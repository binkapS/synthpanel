//! Per-site SSH users.
//!
//! A site user is a real local account whose home is the site document root,
//! so SFTP/SSH access lands directly in the files the site serves. Accounts
//! are key-only (`passwd --lock`), are never `root`, and are confined to a
//! document root inside the configured web directory. The panel records the
//! account so it can list and remove it later.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::config::Paths;

/// Names that must never be provisioned as a site user.
const RESERVED: &[&str] = &[
    "root",
    "admin",
    "administrator",
    "operator",
    "panel",
    "synthpanel",
    "www-data",
    "www",
    "web",
    "nginx",
    "apache",
    "mysql",
    "mariadb",
    "postgres",
    "pgsql",
    "redis",
    "daemon",
    "bin",
    "sys",
    "sync",
    "games",
    "man",
    "lp",
    "mail",
    "news",
    "uucp",
    "proxy",
    "backup",
    "list",
    "irc",
    "gnats",
    "nobody",
    "systemd-network",
    "systemd-resolve",
    "messagebus",
    "sshd",
    "ubuntu",
    "debian",
    "git",
    "ftp",
    "postfix",
    "dovecot",
];

/// Validate a local username.
///
/// Conservative Linux account rules: lowercase start, no dots or slashes,
/// bounded length, and an explicit denylist that includes `root`.
pub fn validate_username(username: &str) -> Result<String> {
    let username = username.trim();
    if username.is_empty() || username.len() > 32 {
        bail!("the username must be between 1 and 32 characters long");
    }
    let mut chars = username.chars();
    let first = chars.next().expect("non-empty username");
    if !first.is_ascii_lowercase() {
        bail!("the username must start with a lowercase letter");
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-') {
        bail!("the username may only contain lowercase letters, digits, underscores and dashes");
    }
    if RESERVED.contains(&username) {
        bail!("the username `{username}` is reserved");
    }
    Ok(username.to_string())
}

/// Normalise and validate a site document root.
///
/// The root must be absolute, free of `..`, and inside the configured web
/// directory. This prevents a site user from being given a home directory
/// such as `/etc` or `/root`.
pub fn validate_document_root(root: &Path, www_dir: &Path) -> Result<PathBuf> {
    if !root.is_absolute() {
        bail!("the site document root must be an absolute path");
    }
    if !www_dir.is_absolute() || www_dir == Path::new("/") {
        bail!("the configured web directory must be an absolute path other than `/`");
    }
    let mut normalised = PathBuf::new();
    for component in root.components() {
        match component {
            Component::RootDir => normalised.push("/"),
            Component::Normal(part) => normalised.push(part),
            Component::CurDir => {}
            Component::ParentDir => bail!("the site document root must not contain `..`"),
            Component::Prefix(_) => bail!("the site document root must be a Unix path"),
        }
    }
    if normalised == Path::new("/") {
        bail!("the site document root must not be `/`");
    }
    if !normalised.starts_with(www_dir) {
        bail!(
            "the site document root must be inside {}",
            www_dir.display()
        );
    }
    Ok(normalised)
}

/// Where a site user's authorized keys live.
pub fn authorized_keys_path(document_root: &Path) -> PathBuf {
    document_root.join(".ssh").join("authorized_keys")
}

/// Whether a local account already exists, according to `/etc/passwd`.
pub fn user_exists(username: &str) -> bool {
    std::fs::read_to_string("/etc/passwd")
        .map(|passwd| {
            passwd
                .lines()
                .any(|line| line.split(':').next() == Some(username))
        })
        .unwrap_or(false)
}

/// Resolve a system binary with an environment override (used by tests and
/// by distros that move these tools).
fn bin(name: &str, env_key: &str) -> String {
    std::env::var(env_key).unwrap_or_else(|_| name.to_string())
}

/// Run a system command with explicit arguments (never through a shell).
fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("cannot run `{program}`; is it installed?"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Set ownership of a path to the newly created account.
fn chown_to(username: &str, path: &Path) -> Result<()> {
    let owner = format!("{username}:{username}");
    let target = path.to_string_lossy().to_string();
    run(&bin("chown", "SYNTHPANEL_CHOWN_BIN"), &[&owner, &target])
}

/// Validate a single-line OpenSSH public key and return its trimmed form.
pub fn validate_public_key(public_key: &str) -> Result<String> {
    crate::security::validate_ssh_key(public_key)?;
    let key = public_key.trim();
    if key.chars().any(char::is_control) {
        bail!("the public key must be a single line");
    }
    Ok(key.to_string())
}

/// Install a public key for a site user.
pub fn install_key(username: &str, document_root: &Path, public_key: &str) -> Result<()> {
    let key = validate_public_key(public_key)?;
    let path = authorized_keys_path(document_root);
    let ssh_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid authorized_keys path"))?;
    std::fs::create_dir_all(ssh_dir)
        .with_context(|| format!("cannot create {}", ssh_dir.display()))?;
    std::fs::set_permissions(ssh_dir, std::fs::Permissions::from_mode(0o700))?;

    let mut contents = std::fs::read_to_string(&path).unwrap_or_default();
    if !contents.lines().any(|line| line.trim() == key) {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(&key);
        contents.push('\n');
    }
    std::fs::write(&path, &contents).with_context(|| format!("cannot write {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    chown_to(username, ssh_dir)?;
    chown_to(username, &path)?;
    Ok(())
}

/// Remove a public key from a site user's authorized keys.
pub fn remove_key(document_root: &Path, public_key: &str) -> Result<()> {
    let path = authorized_keys_path(document_root);
    if !path.exists() {
        return Ok(());
    }
    let key = public_key.trim();
    let contents = std::fs::read_to_string(&path)?;
    let kept: Vec<&str> = contents.lines().filter(|line| line.trim() != key).collect();
    if kept.is_empty() {
        let _ = std::fs::remove_file(&path);
        if let Some(ssh_dir) = path.parent() {
            let _ = std::fs::remove_dir(ssh_dir);
        }
        return Ok(());
    }
    let mut updated = kept.join("\n");
    updated.push('\n');
    std::fs::write(&path, updated)?;
    Ok(())
}

/// Create a key-only local account scoped to the site document root.
pub fn provision_system_user(username: &str, document_root: &Path, public_key: &str) -> Result<()> {
    let username = validate_username(username)?;
    if user_exists(&username) {
        bail!("the local account `{username}` already exists");
    }
    let home = document_root.to_string_lossy().to_string();
    run(
        &bin("useradd", "SYNTHPANEL_USERADD_BIN"),
        &[
            "--user-group",
            "--no-create-home",
            "--home-dir",
            &home,
            "--shell",
            "/bin/bash",
            &username,
        ],
    )?;
    // Passwords are never accepted for these accounts: SSH keys only.
    if let Err(error) = run(
        &bin("passwd", "SYNTHPANEL_PASSWD_BIN"),
        &["--lock", &username],
    ) {
        let _ = run(&bin("userdel", "SYNTHPANEL_USERDEL_BIN"), &[&username]);
        return Err(error);
    }
    if let Err(error) = install_key(&username, document_root, public_key) {
        let _ = run(&bin("userdel", "SYNTHPANEL_USERDEL_BIN"), &[&username]);
        return Err(error);
    }
    Ok(())
}

/// Remove the local account and its installed key.
///
/// `userdel` is used without `-r`: the site's files belong to the site, not to
/// the account, so they must survive user removal.
pub fn deprovision_system_user(
    username: &str,
    document_root: &Path,
    public_key: &str,
) -> Result<()> {
    let username = validate_username(username)?;
    remove_key(document_root, public_key)?;
    if user_exists(&username) {
        run(&bin("userdel", "SYNTHPANEL_USERDEL_BIN"), &[&username])?;
    }
    Ok(())
}

/// A stored site SSH user record.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SiteSshUserRecord {
    pub id: String,
    pub site_id: String,
    pub username: String,
    pub document_root: String,
    pub public_key: String,
    pub created_at: String,
}

/// The public view of a site SSH user returned by `site_ssh_users.list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteSshUser {
    pub username: String,
    pub created: String,
}

impl From<&SiteSshUserRecord> for SiteSshUser {
    fn from(record: &SiteSshUserRecord) -> Self {
        Self {
            username: record.username.clone(),
            created: record.created_at.clone(),
        }
    }
}

/// Repository for managed per-site SSH users.
#[derive(Debug, Clone)]
pub struct SiteUsers {
    pool: SqlitePool,
    paths: Paths,
}

impl SiteUsers {
    pub fn new(pool: SqlitePool, paths: Paths) -> Self {
        Self { pool, paths }
    }

    /// Add a site SSH user: provision the local account first, then record it.
    pub async fn add(
        &self,
        site_id: &str,
        username: &str,
        public_key: &str,
    ) -> Result<SiteSshUserRecord> {
        let username = validate_username(username)?;
        validate_public_key(public_key)?;
        if self.find_by_username(&username).await?.is_some() {
            bail!("the site SSH user `{username}` already exists");
        }
        let site = crate::sites::Sites::new(self.pool.clone())
            .get(site_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("site `{site_id}` does not exist"))?;
        let document_root =
            validate_document_root(Path::new(&site.document_root), &self.paths.www_dir)?;

        provision_system_user(&username, &document_root, public_key)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO site_ssh_users
                (id, site_id, username, document_root, public_key, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(site_id)
        .bind(&username)
        .bind(document_root.to_string_lossy().to_string())
        .bind(public_key.trim())
        .bind(&now)
        .execute(&self.pool)
        .await;
        if let Err(error) = result {
            let _ = run(&bin("userdel", "SYNTHPANEL_USERDEL_BIN"), &[&username]);
            return Err(error.into());
        }

        self.get(site_id, &username)
            .await?
            .ok_or_else(|| anyhow::anyhow!("site SSH user vanished after insert"))
    }

    /// List the public view of a site's SSH users.
    pub async fn list(&self, site_id: &str) -> Result<Vec<SiteSshUser>> {
        Ok(self
            .list_records(site_id)
            .await?
            .iter()
            .map(SiteSshUser::from)
            .collect())
    }

    /// List full records (including the public key) for a site.
    pub async fn list_records(&self, site_id: &str) -> Result<Vec<SiteSshUserRecord>> {
        let rows = sqlx::query_as::<_, SiteSshUserRecord>(
            "SELECT * FROM site_ssh_users WHERE site_id = ? ORDER BY username",
        )
        .bind(site_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get(&self, site_id: &str, username: &str) -> Result<Option<SiteSshUserRecord>> {
        let row = sqlx::query_as::<_, SiteSshUserRecord>(
            "SELECT * FROM site_ssh_users WHERE site_id = ? AND username = ?",
        )
        .bind(site_id)
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn find_by_username(&self, username: &str) -> Result<Option<SiteSshUserRecord>> {
        let row = sqlx::query_as::<_, SiteSshUserRecord>(
            "SELECT * FROM site_ssh_users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Remove a site SSH user, cleaning up the local account and key first.
    pub async fn remove(&self, site_id: &str, username: &str) -> Result<bool> {
        let record = match self.get(site_id, username).await? {
            Some(record) => record,
            None => return Ok(false),
        };
        deprovision_system_user(
            &record.username,
            Path::new(&record.document_root),
            &record.public_key,
        )?;
        let result = sqlx::query("DELETE FROM site_ssh_users WHERE id = ?")
            .bind(&record.id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKeyMaterial site@example.com";

    #[test]
    fn usernames_are_validated_and_root_is_rejected() {
        assert_eq!(validate_username("site_user1").unwrap(), "site_user1");
        assert_eq!(validate_username("site-user").unwrap(), "site-user");
        assert!(validate_username("root").is_err());
        assert!(validate_username("ROOT").is_err());
        assert!(validate_username("1user").is_err());
        assert!(validate_username("user.name").is_err());
        assert!(validate_username("../etc").is_err());
        assert!(validate_username("www-data").is_err());
        assert!(validate_username("").is_err());
        assert!(validate_username(&"a".repeat(33)).is_err());
    }

    #[test]
    fn document_roots_must_be_scoped_to_the_web_directory() {
        let www = Path::new("/var/www");
        assert_eq!(
            validate_document_root(Path::new("/var/www/shop/public"), www).unwrap(),
            PathBuf::from("/var/www/shop/public")
        );
        assert!(validate_document_root(Path::new("/var/www/shop/./public"), www).is_ok());
        assert!(validate_document_root(Path::new("/var/www/shop/../etc"), www).is_err());
        assert!(validate_document_root(Path::new("/etc/passwd"), www).is_err());
        assert!(validate_document_root(Path::new("var/www/shop"), www).is_err());
        assert!(validate_document_root(Path::new("/"), www).is_err());
    }

    #[test]
    fn authorized_keys_stay_inside_the_document_root() {
        let path = authorized_keys_path(Path::new("/var/www/shop/public"));
        assert_eq!(
            path,
            PathBuf::from("/var/www/shop/public/.ssh/authorized_keys")
        );
    }

    #[test]
    fn key_validation_rejects_garbage() {
        assert_eq!(validate_public_key(KEY).unwrap(), KEY);
        assert!(validate_public_key("not a key").is_err());
        assert!(validate_public_key(&format!("{KEY}\ncommand=\"id\"")).is_err());
    }

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-site-users-{}.db", Uuid::new_v4()));
        let pool = crate::db::connect(&path).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn list_and_remove_never_touch_unknown_accounts() {
        let pool = test_pool().await;
        let mut www = std::env::temp_dir();
        www.push(format!("sp-www-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&www).unwrap();
        let paths = crate::config::Paths {
            www_dir: www.clone(),
            ..crate::config::Paths::from_env()
        };
        let users = SiteUsers::new(pool.clone(), paths);

        // No system calls are made for a record that does not exist.
        assert!(!users.remove("site-1", "missing").await.unwrap());
        assert!(users.list("site-1").await.unwrap().is_empty());

        // Insert a record directly so the test never has to run `useradd`.
        sqlx::query(
            "INSERT INTO site_ssh_users
                (id, site_id, username, document_root, public_key, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind("site-1")
        .bind("shop_deploy")
        .bind(www.join("shop/public").to_string_lossy().to_string())
        .bind(KEY)
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let listed = users.list("site-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].username, "shop_deploy");
        assert!(!listed[0].created.is_empty());
        assert!(
            users
                .find_by_username("shop_deploy")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn add_rejects_root_before_any_system_call() {
        let pool = test_pool().await;
        let paths = crate::config::Paths::from_env();
        let users = SiteUsers::new(pool, paths);
        assert!(users.add("site-1", "root", KEY).await.is_err());
        assert!(users.add("site-1", "bad name", KEY).await.is_err());
    }
}
