//! PHP-FPM pool rendering and version management.

use crate::sites::SiteExtras;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhpOverrides {
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default)]
    pub max_execution_time: Option<i64>,
    #[serde(default)]
    pub upload_max_filesize: Option<String>,
    #[serde(default)]
    pub post_max_size: Option<String>,
    #[serde(default)]
    pub additional: Option<Value>,
}

pub fn parse_overrides(json: &str) -> Result<PhpOverrides> {
    let parsed: Value = serde_json::from_str(json)?;
    Ok(PhpOverrides::deserialize(parsed)?)
}

/// Render a PHP-FPM pool file for a site.
pub fn render_pool(site: &SiteExtras) -> String {
    let extras = site;
    let site = &extras.site;
    let php_version = site.php_version.as_deref().unwrap_or("8.3");
    let overrides_json = extras.php.as_deref().unwrap_or("{}");
    let overrides = parse_overrides(overrides_json).unwrap_or_default();

    let overrides_text = format_overrides(&overrides);
    let mut out = String::new();
    out.push_str(&format!("[sp-{id}]\n", id = site.id));
    out.push_str(&format!(
        "user = {user}\ngroup = {user}\n",
        user = site.pool_user
    ));
    out.push_str(&format!(
        "listen = /run/php/php{ver}-sp-{id}.sock\n",
        ver = php_version,
        id = site.id
    ));
    out.push_str("listen.owner = synthpanel\nlisten.group = synthpanel\nlisten.mode = 0660\n");
    out.push_str(
        "pm = dynamic\npm.max_children = 10\npm.start_servers = 2\npm.min_spare_servers = 1\npm.max_spare_servers = 3\npm.max_requests = 50000\n",
    );
    out.push_str(&overrides_text);
    out
}

fn format_overrides(overrides: &PhpOverrides) -> String {
    let mut out = String::new();
    if let Some(value) = &overrides.memory_limit {
        out.push_str(&format!("php_admin_value[memory_limit] = {value}\n"));
    }
    if let Some(value) = overrides.max_execution_time {
        out.push_str(&format!("php_admin_value[max_execution_time] = {value}\n"));
    }
    if let Some(value) = &overrides.upload_max_filesize {
        out.push_str(&format!("php_admin_value[upload_max_filesize] = {value}\n"));
    }
    if let Some(value) = &overrides.post_max_size {
        out.push_str(&format!("php_admin_value[post_max_size] = {value}\n"));
    }
    out
}

/// PHP versions managed by the panel.
///
/// The catalogue tracks major/minor releases; patch releases of a line (for
/// example 8.4.x) use the corresponding major/minor entry. The binary is
/// discovered at runtime so a version can be present without its package
/// being installed.
pub const PHP_VERSIONS: &[&str] = &["8.1", "8.2", "8.3", "8.4", "8.5"];

/// Environment variable prefix for overriding a version's FPM binary.
///
/// The suffix is the version with dots replaced by underscores, e.g.
/// `SYNTHPANEL_PHP_FPM_8_5=/opt/php/bin/php-fpm8.5`. The override is optional;
/// when absent the binary is looked up on `PATH`.
pub const PHP_FPM_BINARY_ENV_PREFIX: &str = "SYNTHPANEL_PHP_FPM_";

pub fn is_supported_version(version: &str) -> bool {
    PHP_VERSIONS.contains(&version)
}

/// The env key used to override the FPM binary of a version.
pub fn php_fpm_binary_env_key(version: &str) -> String {
    format!("{PHP_FPM_BINARY_ENV_PREFIX}{}", version.replace('.', "_"))
}

/// Candidate FPM binary names for a version, in lookup order.
///
/// Debian and Ubuntu use both spellings (`php-fpm84` and `php-fpm8.4`)
/// depending on the release, so neither is assumed to exist.
pub fn php_fpm_binary_names(version: &str) -> [String; 2] {
    [
        format!("php-fpm{}", version.replace('.', "")),
        format!("php-fpm{version}"),
    ]
}

/// Resolve the FPM binary for a version: explicit env override first, then
/// the conventional names on `PATH`. Returns `None` when nothing is installed.
pub fn php_fpm_binary(version: &str) -> Option<PathBuf> {
    let key = php_fpm_binary_env_key(version);
    if let Some(path) = std::env::var_os(&key).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    for name in php_fpm_binary_names(version) {
        if let Some(path) = which(&name) {
            return Some(path);
        }
    }
    None
}

/// Pool directory for a version. `Paths` only knows the historically
/// supported versions, so newer catalogue entries fall back to the same
/// `SYNTHPANEL_PHP_DIR` override that `Paths::from_env` uses.
fn pool_root(paths: &crate::apply::Paths, version: &str, index: usize) -> PathBuf {
    if let Some(path) = paths.php_pools.get(index) {
        return path.clone();
    }
    let php_dir = std::env::var_os("SYNTHPANEL_PHP_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/synthpanel/php"));
    php_dir.join(version)
}

/// What the panel knows about the PHP installation of this server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PhpStatus {
    /// The versions the panel supports.
    pub versions: Vec<String>,
    /// One entry per supported version.
    pub installed: Vec<PhpVersion>,
    /// The sites that use each version.
    pub sites: Vec<PhpSite>,
}

/// A single PHP version.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PhpVersion {
    pub version: String,
    /// The FPM binary, when it is on the PATH.
    pub binary: Option<String>,
    /// The directory that holds the generated pools.
    pub pool_dir: String,
    /// The pools the panel generated for this version.
    pub pools: Vec<String>,
    /// Whether the service is running.
    pub active: bool,
}

/// A site and the PHP version it runs on.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PhpSite {
    pub id: String,
    pub domain: String,
    pub version: String,
}

/// Collect the PHP status of the server.
pub async fn status(pool: &sqlx::SqlitePool) -> Result<PhpStatus> {
    let paths = crate::apply::Paths::from_env();
    let mut installed = Vec::new();

    for (index, version) in PHP_VERSIONS.iter().enumerate() {
        let binary = php_fpm_binary(version).map(|path| path.to_string_lossy().to_string());
        let pool_dir = pool_root(&paths, version, index);
        let pool_root = pool_dir.join("fpm/pool.d");
        let mut pools = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&pool_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("synthpanel-") && name.ends_with(".conf") {
                    pools.push(name);
                }
            }
        }
        pools.sort();
        installed.push(PhpVersion {
            version: (*version).to_string(),
            binary,
            pool_dir: pool_root.to_string_lossy().to_string(),
            pools,
            active: false,
        });
    }

    let rows = sqlx::query_as::<_, PhpSiteRow>(
        "SELECT id, domain, COALESCE(php_version, '8.3') AS version FROM sites ORDER BY domain",
    )
    .fetch_all(pool)
    .await?;
    let sites = rows
        .into_iter()
        .map(|row| PhpSite {
            id: row.id,
            domain: row.domain,
            version: row.version,
        })
        .collect();

    Ok(PhpStatus {
        versions: PHP_VERSIONS.iter().map(|v| (*v).to_string()).collect(),
        installed,
        sites,
    })
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PhpSiteRow {
    id: String,
    domain: String,
    version: String,
}

/// Find an executable binary on the PATH.
fn which(binary: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.is_file()
            && candidate
                .metadata()
                .map(|meta| meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};

    #[tokio::test]
    async fn reports_the_versions_and_the_sites() {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-php-status-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let sites = Sites::new(pool.clone());
        sites
            .create(CreateSite {
                domain: "example.com".into(),
                aliases: vec![],
                document_root: "/var/www/sp_test/public".into(),
                template: "php".into(),
                php_version: Some("8.2".into()),
                ssl_mode: "none".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap();

        let status = status(&pool).await.unwrap();
        assert_eq!(status.versions, vec!["8.1", "8.2", "8.3", "8.4", "8.5"]);
        assert_eq!(status.installed.len(), PHP_VERSIONS.len());
        assert_eq!(status.sites.len(), 1);
        assert_eq!(status.sites[0].domain, "example.com");
        assert_eq!(status.sites[0].version, "8.2");
    }

    async fn make_site(overrides: &str) -> SiteExtras {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-php-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let sites = Sites::new(pool.clone());
        let site = sites
            .create(CreateSite {
                domain: "example.com".into(),
                aliases: Vec::new(),
                document_root: "/var/www/sp_test".into(),
                template: "php".into(),
                php_version: Some("8.3".into()),
                ssl_mode: "none".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap();
        if !overrides.is_empty() {
            sqlx::query("UPDATE site_php SET overrides_json = ? WHERE site_id = ?")
                .bind(overrides)
                .bind(&site.id)
                .execute(&pool)
                .await
                .unwrap();
            sites.get_with_extras(&site.id).await.unwrap().unwrap()
        } else {
            sites.get_with_extras(&site.id).await.unwrap().unwrap()
        }
    }

    #[tokio::test]
    async fn renders_pool_with_defaults() {
        let site = make_site("{}").await;
        let text = render_pool(&site);
        assert!(text.contains("[sp-"));
        assert!(text.contains("user = sp_"));
        assert!(text.contains("listen = /run/php/php8.3-sp-"));
        assert!(text.contains("pm = dynamic"));
    }

    #[tokio::test]
    async fn renders_pool_with_overrides() {
        let site = make_site(
            r#"{"memory_limit": "512M", "max_execution_time": 90, "upload_max_filesize": "64M"}"#,
        )
        .await;
        let text = render_pool(&site);
        assert!(text.contains("php_admin_value[memory_limit] = 512M"));
        assert!(text.contains("php_admin_value[max_execution_time] = 90"));
        assert!(text.contains("php_admin_value[upload_max_filesize] = 64M"));
    }

    #[test]
    fn supports_expected_php_versions() {
        assert!(is_supported_version("8.3"));
        assert!(is_supported_version("8.4"));
        assert!(is_supported_version("8.5"));
        assert!(!is_supported_version("7.4"));
        assert_eq!(PHP_VERSIONS.len(), 5);
    }

    #[test]
    fn fpm_binary_lookup_is_overridable() {
        assert_eq!(php_fpm_binary_env_key("8.5"), "SYNTHPANEL_PHP_FPM_8_5");
        assert_eq!(php_fpm_binary_env_key("8.4"), "SYNTHPANEL_PHP_FPM_8_4");
        assert_eq!(php_fpm_binary_names("8.4"), ["php-fpm84", "php-fpm8.4"]);
        assert_eq!(php_fpm_binary_names("8.5"), ["php-fpm85", "php-fpm8.5"]);
    }

    #[test]
    fn newer_versions_get_a_pool_dir() {
        let paths = crate::apply::Paths::from_env();
        let base = std::env::var_os("SYNTHPANEL_PHP_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/synthpanel/php"));
        assert_eq!(pool_root(&paths, "8.5", 4), base.join("8.5"));
    }

    #[test]
    fn overrides_roundtrip() {
        let overrides = PhpOverrides {
            memory_limit: Some("256M".into()),
            max_execution_time: Some(45),
            upload_max_filesize: None,
            post_max_size: None,
            additional: None,
        };
        let json = serde_json::to_string(&overrides).unwrap();
        let back = parse_overrides(&json).unwrap();
        assert_eq!(back.memory_limit.as_deref(), Some("256M"));
        assert_eq!(back.max_execution_time, Some(45));
    }
}
