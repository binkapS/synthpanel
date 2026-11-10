//! PHP-FPM pool rendering and version management.

use crate::sites::SiteExtras;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub const PHP_VERSIONS: &[&str] = &["8.1", "8.2", "8.3", "8.4"];

pub fn is_supported_version(version: &str) -> bool {
    PHP_VERSIONS.contains(&version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};

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
        assert!(!is_supported_version("7.4"));
        assert_eq!(PHP_VERSIONS.len(), 4);
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
