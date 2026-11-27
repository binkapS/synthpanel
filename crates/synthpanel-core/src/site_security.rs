//! Per-site security controls: IP blocks, bot-name blocks, HTTP Basic Auth
//! and a Cloudflare-only access rule.
//!
//! The settings are stored in their own table so sites without overrides keep
//! the permissive defaults. The Basic Auth password is never returned over
//! IPC: only the htpasswd-format hash is persisted and written to the file
//! referenced by the generated vhost.

use anyhow::{Context, Result};
use base64::Engine;
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use std::path::{Path, PathBuf};

/// The public shape of a site's security settings (the `site_security.*` IPC
/// contract). Extra keys in the params object, such as `site_id`, are ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteSecuritySettings {
    pub ip_blocks: Vec<String>,
    pub bot_blocks: Vec<String>,
    pub basic_auth_enabled: bool,
    pub basic_auth_user: String,
    pub cloudflare_only: bool,
}

impl SiteSecuritySettings {
    /// Parse settings from an IPC params object.
    pub fn from_params(params: &Value) -> Result<Self> {
        serde_json::from_value(params.clone()).context("invalid site security settings")
    }

    /// Whether any access control needs to be rendered into the vhost.
    pub fn has_restrictions(&self) -> bool {
        self.cloudflare_only
            || self.basic_auth_enabled
            || !self.ip_blocks.is_empty()
            || !self.bot_blocks.is_empty()
    }
}

#[derive(Debug, FromRow)]
struct SiteSecurityRow {
    ip_blocks: String,
    bot_blocks: String,
    basic_auth_enabled: i64,
    basic_auth_user: String,
    basic_auth_password_hash: String,
    cloudflare_only: i64,
}

/// Repository for `site_security`.
#[derive(Debug, Clone)]
pub struct SiteSecurity {
    pool: SqlitePool,
}

impl SiteSecurity {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The settings of a site, or the permissive defaults when it has no row.
    pub async fn get(&self, site_id: &str) -> Result<SiteSecuritySettings> {
        get_site_security(&self.pool, site_id).await
    }

    /// The settings together with the stored htpasswd hash (used by `set` to
    /// preserve a password when the caller does not submit a new one).
    pub async fn get_record(&self, site_id: &str) -> Result<(SiteSecuritySettings, String)> {
        get_site_security_record(&self.pool, site_id).await
    }

    /// Persist settings and refresh the per-site htpasswd file.
    pub async fn set(
        &self,
        site_id: &str,
        settings: &SiteSecuritySettings,
        password: Option<&str>,
    ) -> Result<()> {
        set_site_security(&self.pool, site_id, settings, password).await
    }
}

/// Read a site's security settings (defaults when no row exists).
pub async fn get_site_security(pool: &SqlitePool, site_id: &str) -> Result<SiteSecuritySettings> {
    Ok(get_site_security_record(pool, site_id).await?.0)
}

/// Read settings plus the stored htpasswd hash.
pub async fn get_site_security_record(
    pool: &SqlitePool,
    site_id: &str,
) -> Result<(SiteSecuritySettings, String)> {
    let row: Option<SiteSecurityRow> = sqlx::query_as(
        "SELECT ip_blocks, bot_blocks, basic_auth_enabled, basic_auth_user,
                basic_auth_password_hash, cloudflare_only
         FROM site_security WHERE site_id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    match row {
        None => Ok((SiteSecuritySettings::default(), String::new())),
        Some(row) => Ok((
            SiteSecuritySettings {
                ip_blocks: serde_json::from_str(&row.ip_blocks).unwrap_or_default(),
                bot_blocks: serde_json::from_str(&row.bot_blocks).unwrap_or_default(),
                basic_auth_enabled: row.basic_auth_enabled != 0,
                basic_auth_user: row.basic_auth_user,
                cloudflare_only: row.cloudflare_only != 0,
            },
            row.basic_auth_password_hash,
        )),
    }
}

/// Persist a site's security settings and refresh its htpasswd file.
///
/// `password` is optional because the `site_security.set` contract does not
/// carry one; when absent the previously stored hash is kept. The file is
/// written before the row is committed so a failed DB write cannot leave the
/// vhost pointing at a stale credential file.
pub async fn set_site_security(
    pool: &SqlitePool,
    site_id: &str,
    settings: &SiteSecuritySettings,
    password: Option<&str>,
) -> Result<()> {
    set_site_security_in(pool, &htpasswd_root(), site_id, settings, password).await
}

/// Same as [`set_site_security`] with an explicit snippets directory.
pub async fn set_site_security_in(
    pool: &SqlitePool,
    root: &Path,
    site_id: &str,
    settings: &SiteSecuritySettings,
    password: Option<&str>,
) -> Result<()> {
    let (_, old_hash) = get_site_security_record(pool, site_id).await?;
    let hash = match password {
        Some(password) if !password.trim().is_empty() => hash_password(password),
        _ => old_hash,
    };
    write_htpasswd_in(root, site_id, auth_user(settings), &hash)?;

    let ip_blocks = serde_json::to_string(&settings.ip_blocks)?;
    let bot_blocks = serde_json::to_string(&settings.bot_blocks)?;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO site_security
            (site_id, ip_blocks, bot_blocks, basic_auth_enabled, basic_auth_user,
             basic_auth_password_hash, cloudflare_only, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(site_id) DO UPDATE SET
             ip_blocks = excluded.ip_blocks,
             bot_blocks = excluded.bot_blocks,
             basic_auth_enabled = excluded.basic_auth_enabled,
             basic_auth_user = excluded.basic_auth_user,
             basic_auth_password_hash = excluded.basic_auth_password_hash,
             cloudflare_only = excluded.cloudflare_only,
             updated_at = excluded.updated_at",
    )
    .bind(site_id)
    .bind(&ip_blocks)
    .bind(&bot_blocks)
    .bind(i64::from(settings.basic_auth_enabled))
    .bind(&settings.basic_auth_user)
    .bind(&hash)
    .bind(i64::from(settings.cloudflare_only))
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Hash a Basic Auth password in the `htpasswd` `{SHA}` scheme that nginx
/// understands (`user:{SHA}<base64(SHA-1(password))>`).
pub fn hash_password(password: &str) -> String {
    let sha1 = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, password.as_bytes());
    format!(
        "{{SHA}}{}",
        base64::engine::general_purpose::STANDARD.encode(sha1.as_ref())
    )
}

fn auth_user(settings: &SiteSecuritySettings) -> &str {
    let user = settings.basic_auth_user.trim();
    if user.is_empty() { "synthpanel" } else { user }
}

/// Root directory for generated `auth_basic_user_file` files.
///
/// It follows `SYNTHPANEL_NGINX_DIR` through the same `apply::Paths` helper the
/// vhost writer uses, so generated vhosts and credential files stay together
/// in development containers and in production.
pub fn htpasswd_root() -> PathBuf {
    crate::apply::Paths::from_env().snippets
}

pub fn htpasswd_path_in(root: &Path, site_id: &str) -> PathBuf {
    root.join(format!("htpasswd-{site_id}"))
}

pub fn htpasswd_path(site_id: &str) -> PathBuf {
    htpasswd_path_in(&htpasswd_root(), site_id)
}

/// Write (or replace) the htpasswd file for a site.
pub fn write_htpasswd_in(root: &Path, site_id: &str, user: &str, hash: &str) -> Result<PathBuf> {
    let path = htpasswd_path_in(root, site_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let content = if hash.is_empty() {
        String::new()
    } else {
        format!("{user}:{hash}\n")
    };
    std::fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
    Ok(path)
}

pub fn write_htpasswd(site_id: &str, user: &str, hash: &str) -> Result<PathBuf> {
    write_htpasswd_in(&htpasswd_root(), site_id, user, hash)
}

/// Webroot served for ACME HTTP-01 challenges (mirrors the IPC SSL handler).
pub fn acme_webroot(site_id: &str) -> PathBuf {
    crate::config::Paths::from_env().www_dir.join(site_id)
}

/// Cloudflare's published origin IP ranges. These are checked first so that
/// `deny all` only affects traffic that did not come through Cloudflare.
/// They need to be refreshed when Cloudflare changes its ranges.
pub const CLOUDFLARE_RANGES: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

/// Render the server-level access directives for a site.
pub fn render_security_block(settings: &SiteSecuritySettings, site_id: &str) -> String {
    render_security_block_to(settings, &htpasswd_path(site_id))
}

fn render_security_block_to(settings: &SiteSecuritySettings, htpasswd: &Path) -> String {
    let mut out = String::new();

    // Explicit blocks come first so a blocked address is denied even when it
    // falls inside a Cloudflare range.
    let ips: Vec<&str> = settings
        .ip_blocks
        .iter()
        .map(|value| value.trim())
        .filter(|value| safe_ip(value))
        .collect();
    if !ips.is_empty() {
        out.push_str("    # Blocked IP addresses\n");
        for ip in ips {
            out.push_str(&format!("    deny {ip};\n"));
        }
    }

    if settings.cloudflare_only {
        out.push_str("    # Cloudflare-only access\n");
        for range in CLOUDFLARE_RANGES {
            out.push_str(&format!("    allow {range};\n"));
        }
        out.push_str("    deny all;\n");
    }

    let bots: Vec<String> = settings
        .bot_blocks
        .iter()
        .map(|value| value.trim())
        .filter(|value| safe_bot(value))
        .map(escape_regex)
        .collect();
    if !bots.is_empty() {
        out.push_str("    # Blocked user agents\n");
        out.push_str(&format!(
            "    if ($http_user_agent ~* \"{}\") {{ return 403; }}\n",
            bots.join("|")
        ));
    }

    if settings.basic_auth_enabled {
        out.push_str("    # Per-site Basic Auth\n");
        out.push_str("    auth_basic \"Restricted area\";\n");
        out.push_str(&format!(
            "    auth_basic_user_file {};\n",
            escape_config_path(htpasswd)
        ));
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn safe_ip(value: &str) -> bool {
    value.eq_ignore_ascii_case("all")
        || (!value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '/')))
}

fn safe_bot(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| !c.is_control() && !matches!(c, ';' | '{' | '}' | '#' | '"' | '\\'))
}

fn escape_regex(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '+'
                | '*'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '^'
                | '$'
                | '|'
                | '\\'
                | '/'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn escape_config_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};
    use serde_json::json;

    #[test]
    fn settings_default_to_permissive() {
        let settings = SiteSecuritySettings::from_params(&json!({ "site_id": "abc" })).unwrap();
        assert!(settings.ip_blocks.is_empty());
        assert!(settings.bot_blocks.is_empty());
        assert!(!settings.basic_auth_enabled);
        assert!(!settings.cloudflare_only);
        assert!(!settings.has_restrictions());
    }

    #[test]
    fn settings_parse_from_params() {
        let settings = SiteSecuritySettings::from_params(&json!({
            "site_id": "abc",
            "ip_blocks": ["203.0.113.4", "10.0.0.0/8"],
            "bot_blocks": ["BadBot/1.0"],
            "basic_auth_enabled": true,
            "basic_auth_user": "admin",
            "cloudflare_only": true,
        }))
        .unwrap();
        assert_eq!(settings.ip_blocks, vec!["203.0.113.4", "10.0.0.0/8"]);
        assert_eq!(settings.basic_auth_user, "admin");
        assert!(settings.basic_auth_enabled);
        assert!(settings.cloudflare_only);
        assert!(settings.has_restrictions());
    }

    #[test]
    fn nginx_sha_scheme_matches_htpasswd() {
        // The htpasswd(1) -s output for "password123".
        assert_eq!(
            hash_password("password123"),
            "{SHA}y/2sYAj5yrQIN4TL0YdPdmGNKpc="
        );
    }

    #[test]
    fn renders_all_security_controls() {
        let settings = SiteSecuritySettings {
            ip_blocks: vec!["203.0.113.10".into(), "bad; deny all;".into()],
            bot_blocks: vec!["BadBot/1.0".into()],
            basic_auth_enabled: true,
            basic_auth_user: "admin".into(),
            cloudflare_only: true,
        };
        let text =
            render_security_block_to(&settings, Path::new("/opt/sp/nginx/snippets/htpasswd-abc"));
        assert!(text.contains("deny 203.0.113.10;"));
        assert!(!text.contains("bad; deny all;"));
        assert!(text.contains("allow 173.245.48.0/20;"));
        assert!(text.contains("allow 2606:4700::/32;"));
        assert!(text.contains("deny all;"));
        assert!(text.contains(r#"$http_user_agent ~* "BadBot\/1\.0""#));
        assert!(text.contains(r#"auth_basic "Restricted area";"#));
        assert!(text.contains("auth_basic_user_file /opt/sp/nginx/snippets/htpasswd-abc;"));
    }

    #[test]
    fn default_settings_render_nothing() {
        let text = render_security_block_to(&SiteSecuritySettings::default(), Path::new("/x"));
        assert!(text.is_empty());
    }

    #[test]
    fn writes_an_htpasswd_file() {
        let dir = std::env::temp_dir().join(format!("sp-security-{}", uuid::Uuid::new_v4()));
        let path = write_htpasswd_in(&dir, "abc", "admin", "{SHA}abcdef").unwrap();
        assert_eq!(path, htpasswd_path_in(&dir, "abc"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "admin:{SHA}abcdef\n"
        );
        assert_eq!(
            std::fs::read_to_string(write_htpasswd_in(&dir, "abc", "admin", "").unwrap()).unwrap(),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn store_roundtrip_preserves_the_password() {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-security-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let sites = Sites::new(pool.clone());
        let site = sites
            .create(CreateSite {
                domain: "example.com".into(),
                aliases: vec![],
                document_root: "/var/www/sp_test/public".into(),
                template: "php".into(),
                php_version: Some("8.3".into()),
                ssl_mode: "none".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap();
        let snippets =
            std::env::temp_dir().join(format!("sp-sec-snippets-{}", uuid::Uuid::new_v4()));
        let store = SiteSecurity::new(pool.clone());
        assert!(!store.get(&site.id).await.unwrap().has_restrictions());

        let settings = SiteSecuritySettings {
            ip_blocks: vec!["192.0.2.1".into()],
            bot_blocks: vec![],
            basic_auth_enabled: true,
            basic_auth_user: "admin".into(),
            cloudflare_only: false,
        };
        set_site_security_in(&pool, &snippets, &site.id, &settings, Some("secret"))
            .await
            .unwrap();
        let (loaded, hash) = store.get_record(&site.id).await.unwrap();
        assert_eq!(loaded.ip_blocks, vec!["192.0.2.1"]);
        assert_eq!(hash, hash_password("secret"));
        assert!(htpasswd_path_in(&snippets, &site.id).exists());

        // A settings-only update must not drop the password.
        set_site_security_in(&pool, &snippets, &site.id, &settings, None)
            .await
            .unwrap();
        assert_eq!(store.get_record(&site.id).await.unwrap().1, hash);
        let _ = std::fs::remove_dir_all(&snippets);
        let _ = std::fs::remove_file(path);
    }
}
