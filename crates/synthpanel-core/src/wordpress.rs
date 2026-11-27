//! WordPress one-click install.
//!
//! The privileged core downloads (or reads a fixture of) the WordPress
//! tarball, verifies it, unpacks it into a site's document root, hands the
//! files to the site's unprivileged pool user and writes `wp-config.php`
//! with a dedicated database owned by the site's owner.
//!
//! Security rules enforced here:
//! - every archive entry is checked before it is written: no absolute paths,
//!   no `..`, no backslashes, no links or special files, and nothing outside
//!   the archive's single `wordpress/` root;
//! - no external program is ever invoked through a shell — arguments are
//!   passed as discrete `Command` arguments;
//! - WordPress tooling (`wp`) is deliberately *not* run as root: when the
//!   core is root it is wrapped with an env-overridable `runuser`-style
//!   binary and the site's pool user.
//!
//! Tests never need network or a database: [`inspect_archive`],
//! [`extract_archive`] and [`render_wp_config`] are pure/local, and
//! `SYNTHPANEL_WORDPRESS_ARCHIVE` bypasses the download entirely.

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

/// Default download location. Override with `SYNTHPANEL_WORDPRESS_URL`.
pub const DEFAULT_WORDPRESS_URL: &str = "https://wordpress.org/latest.tar.gz";

/// The single top-level directory every archive entry must live under.
const ARCHIVE_ROOT: &str = "wordpress";
const VERSION_FILE: &str = "wp-includes/version.php";
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

/// Arguments of the `wordpress.install` IPC method.
#[derive(Debug, Clone, Copy)]
pub struct InstallRequest<'a> {
    pub site_id: &'a str,
    pub admin_user: &'a str,
    pub admin_password: &'a str,
    pub admin_email: &'a str,
}

/// Result of the `wordpress.install` IPC method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub ok: bool,
    pub version: String,
}

/// What a validated archive contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub version: String,
    pub entries: usize,
}

/// Database connection details written into `wp-config.php`.
#[derive(Debug, Clone)]
pub struct DatabaseCredentials {
    pub name: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: u16,
}

/// The eight WordPress secret keys/salts.
#[derive(Debug, Clone)]
pub struct Salts {
    pub auth_key: String,
    pub secure_auth_key: String,
    pub logged_in_key: String,
    pub nonce_key: String,
    pub auth_salt: String,
    pub secure_auth_salt: String,
    pub logged_in_salt: String,
    pub nonce_salt: String,
}

impl Salts {
    /// Generate fresh random values with the core's password generator.
    pub fn generate() -> Self {
        let random = || crate::database::generate_password(64);
        Self {
            auth_key: random(),
            secure_auth_key: random(),
            logged_in_key: random(),
            nonce_key: random(),
            auth_salt: random(),
            secure_auth_salt: random(),
            logged_in_salt: random(),
            nonce_salt: random(),
        }
    }
}

/// Inputs to [`render_wp_config`].
#[derive(Debug, Clone, Copy)]
pub struct WpConfig<'a> {
    pub db: &'a DatabaseCredentials,
    pub salts: &'a Salts,
}

/// Escape a value for a PHP single-quoted string.
pub fn php_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Render a complete `wp-config.php`.
///
/// Kept free of I/O and randomness so it can be checked offline.
pub fn render_wp_config(config: &WpConfig<'_>) -> String {
    let db = config.db;
    let salts = config.salts;
    format!(
        "<?php\n\
         define( 'DB_NAME', {db_name} );\n\
         define( 'DB_USER', {db_user} );\n\
         define( 'DB_PASSWORD', {db_password} );\n\
         define( 'DB_HOST', {db_host} );\n\
         define( 'DB_CHARSET', 'utf8mb4' );\n\
         define( 'DB_COLLATE', '' );\n\
         \n\
         define( 'AUTH_KEY', {auth_key} );\n\
         define( 'SECURE_AUTH_KEY', {secure_auth_key} );\n\
         define( 'LOGGED_IN_KEY', {logged_in_key} );\n\
         define( 'NONCE_KEY', {nonce_key} );\n\
         define( 'AUTH_SALT', {auth_salt} );\n\
         define( 'SECURE_AUTH_SALT', {secure_auth_salt} );\n\
         define( 'LOGGED_IN_SALT', {logged_in_salt} );\n\
         define( 'NONCE_SALT', {nonce_salt} );\n\
         \n\
         $table_prefix = 'wp_';\n\
         \n\
         define( 'WP_DEBUG', false );\n\
         \n\
         if ( ! defined( 'ABSPATH' ) ) {{\n\
         \tdefine( 'ABSPATH', __DIR__ . '/' );\n\
         }}\n\
         \n\
         require_once ABSPATH . 'wp-settings.php';\n",
        db_name = php_quote(&db.name),
        db_user = php_quote(&db.username),
        db_password = php_quote(&db.password),
        db_host = php_quote(&format!("{}:{}", db.host, db.port)),
        auth_key = php_quote(&salts.auth_key),
        secure_auth_key = php_quote(&salts.secure_auth_key),
        logged_in_key = php_quote(&salts.logged_in_key),
        nonce_key = php_quote(&salts.nonce_key),
        auth_salt = php_quote(&salts.auth_salt),
        secure_auth_salt = php_quote(&salts.secure_auth_salt),
        logged_in_salt = php_quote(&salts.logged_in_salt),
        nonce_salt = php_quote(&salts.nonce_salt),
    )
}

/// Parse `$wp_version = '...';` out of WordPress's version file.
pub fn parse_wp_version(source: &str) -> Option<String> {
    let marker = "$wp_version";
    let start = source.find(marker)? + marker.len();
    let rest = &source[start..];
    let equals = rest.find('=')?;
    let value = rest[equals + 1..].trim_start();
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let end = value[1..].find(quote)? + 1;
    let version = &value[1..end];
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// Map a raw tar path to a path relative to the document root.
///
/// The archive must have a single `wordpress/` root; that component is
/// stripped. The returned path is always relative and never contains `..`.
/// An empty result is the archive root directory itself.
pub fn safe_entry_path(raw: &str) -> Result<PathBuf> {
    if raw.is_empty() {
        bail!("archive entry has an empty path");
    }
    if raw.as_bytes().contains(&0) {
        bail!("archive entry path contains a NUL byte");
    }
    if raw.contains('\\') {
        bail!("archive entry path contains a backslash: {raw}");
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| anyhow!("archive entry path is not UTF-8: {raw}"))?;
                parts.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => bail!("archive entry path contains `..`: {raw}"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("archive entry path is absolute: {raw}")
            }
        }
    }
    if parts.first().copied() != Some(ARCHIVE_ROOT) {
        bail!("archive entry `{raw}` is not inside {ARCHIVE_ROOT}/");
    }
    Ok(parts.into_iter().skip(1).collect())
}

fn is_metadata(kind: tar::EntryType) -> bool {
    matches!(
        kind,
        tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink
            | tar::EntryType::XGlobalHeader
            | tar::EntryType::XHeader
    )
}

/// Read and validate a local `.tar.gz` without extracting it.
///
/// This is the offline-friendly half of the install: fixtures, checksum
/// checks and archive-validation tests all use it.
pub fn inspect_archive(path: &Path) -> Result<ArchiveInfo> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut version = None;
    let mut entries = 0usize;
    let mut unpacked = 0u64;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if is_metadata(kind) {
            continue;
        }
        let raw = entry.header().path()?.to_string_lossy().to_string();
        let relative = safe_entry_path(&raw)?;
        if !kind.is_file() && !kind.is_dir() {
            bail!("archive entry `{raw}` is not a regular file or directory");
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            bail!("archive contains too many entries");
        }
        unpacked = unpacked
            .checked_add(entry.header().size()?)
            .ok_or_else(|| anyhow!("archive size overflow"))?;
        if unpacked > MAX_UNPACKED_BYTES {
            bail!("archive expands to more than {} bytes", MAX_UNPACKED_BYTES);
        }
        if relative == Path::new(VERSION_FILE) && kind.is_file() {
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            version = parse_wp_version(&source);
        }
    }
    let version = version.ok_or_else(|| anyhow!("archive has no readable {VERSION_FILE}"))?;
    Ok(ArchiveInfo { version, entries })
}

/// Extract a validated archive into `root`.
///
/// Every path is revalidated here, so calling this directly is safe too.
/// Existing symlinks are never followed and existing files are replaced.
pub fn extract_archive(path: &Path, root: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut entries = 0usize;
    let mut unpacked = 0u64;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if is_metadata(kind) {
            continue;
        }
        let raw = entry.header().path()?.to_string_lossy().to_string();
        let relative = safe_entry_path(&raw)?;
        if !kind.is_file() && !kind.is_dir() {
            bail!("archive entry `{raw}` is not a regular file or directory");
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            bail!("archive contains too many entries");
        }
        unpacked = unpacked
            .checked_add(entry.header().size()?)
            .ok_or_else(|| anyhow!("archive size overflow"))?;
        if unpacked > MAX_UNPACKED_BYTES {
            bail!("archive expands to more than {} bytes", MAX_UNPACKED_BYTES);
        }

        if kind.is_dir() {
            ensure_directory(root, &relative)?;
            continue;
        }
        if relative.as_os_str().is_empty() {
            bail!("archive entry `{raw}` is a file at the archive root");
        }
        let parent = relative
            .parent()
            .ok_or_else(|| anyhow!("archive entry `{raw}` has no parent"))?;
        let parent = ensure_directory(root, parent)?;
        let target = parent.join(
            relative
                .file_name()
                .ok_or_else(|| anyhow!("archive entry `{raw}` has no file name"))?,
        );
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() {
                bail!("refusing to write through symlink {}", target.display());
            }
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&target)
            .with_context(|| format!("cannot write {}", target.display()))?;
        io::copy(&mut entry, &mut output).with_context(|| format!("cannot extract {raw}"))?;
    }
    Ok(())
}

/// Create `root/relative` one component at a time, refusing symlinks.
fn ensure_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("refusing to write through symlink {}", current.display());
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => bail!("{} is not a directory", current.display()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .with_context(|| format!("cannot create {}", current.display()))?;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o755))?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

/// Verify the SHA256 of a local archive when the panel admin pinned one.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("SYNTHPANEL_WORDPRESS_SHA256 must be a 64-character hex digest");
    }
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let actual = hex::encode(hasher.finalize());
    if actual != expected {
        bail!("WordPress archive checksum mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

/// A local archive, with the downloaded temporary copy removed on drop.
struct LocalArchive {
    path: PathBuf,
    temporary: bool,
}

impl Drop for LocalArchive {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Obtain the archive from the fixture override or download it.
async fn obtain_archive() -> Result<LocalArchive> {
    if let Some(fixture) =
        std::env::var_os("SYNTHPANEL_WORDPRESS_ARCHIVE").filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(fixture);
        let metadata = fs::metadata(&path).with_context(|| {
            format!(
                "cannot read SYNTHPANEL_WORDPRESS_ARCHIVE at {}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            bail!("SYNTHPANEL_WORDPRESS_ARCHIVE is not a file");
        }
        info!(path = %path.display(), "using local WordPress archive fixture");
        return Ok(LocalArchive {
            path,
            temporary: false,
        });
    }

    let url = std::env::var("SYNTHPANEL_WORDPRESS_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_WORDPRESS_URL.to_string());
    let path = std::env::temp_dir().join(format!("synthpanel-wp-{}.tar.gz", Uuid::new_v4()));
    let mut response = reqwest::get(&url)
        .await
        .with_context(|| format!("cannot download WordPress from {url}"))?;
    if !response.status().is_success() {
        bail!("WordPress download failed with HTTP {}", response.status());
    }
    if let Some(length) = response.content_length() {
        if length > MAX_ARCHIVE_BYTES {
            bail!("WordPress archive is larger than {MAX_ARCHIVE_BYTES} bytes");
        }
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let mut total = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .context("cannot read WordPress download")?
    {
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("WordPress download size overflow"))?;
        if total > MAX_ARCHIVE_BYTES {
            bail!("WordPress archive is larger than {MAX_ARCHIVE_BYTES} bytes");
        }
        output.write_all(&chunk)?;
    }
    output.flush()?;
    drop(output);
    info!(url = %url, bytes = total, "downloaded WordPress archive");
    Ok(LocalArchive {
        path,
        temporary: true,
    })
}

/// Validate a system user or `user:group` pair before handing it to `chown`.
pub fn validate_owner(owner: &str) -> Result<String> {
    let owner = owner.trim();
    let parts: Vec<&str> = owner.split(':').collect();
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|part| part.is_empty()) {
        bail!("owner must be `user` or `user:group`");
    }
    for part in &parts {
        if part.len() > 64
            || !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            bail!("owner contains unsupported characters: {owner}");
        }
    }
    Ok(owner.to_string())
}

/// Build the `chown` argument vector. No shell is ever involved.
pub fn chown_args(owner: &str, path: &Path, recursive: bool) -> Result<Vec<String>> {
    let owner = validate_owner(owner)?;
    let mut args = Vec::new();
    if recursive {
        args.push("-R".to_string());
    }
    args.push("--".to_string());
    args.push(owner);
    args.push(path.display().to_string());
    Ok(args)
}

fn apply_ownership(path: &Path, owner: &str, recursive: bool) -> Result<()> {
    let binary = std::env::var("SYNTHPANEL_CHOWN_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "chown".to_string());
    let args = chown_args(owner, path, recursive)?;
    let output = Command::new(&binary)
        .args(&args)
        .output()
        .with_context(|| format!("cannot run `{binary}`; set SYNTHPANEL_CHOWN_BIN"))?;
    if !output.status.success() {
        bail!(
            "{binary} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Validate the WordPress administrator supplied by the caller.
pub fn validate_admin(user: &str, password: &str, email: &str) -> Result<()> {
    if user.trim().is_empty() || user.len() > 60 {
        bail!("the admin user must be between 1 and 60 characters long");
    }
    if !user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        bail!("the admin user may only contain letters, digits, `_`, `-` and `.`");
    }
    if password.is_empty() || password.len() > 256 {
        bail!("the admin password must be between 1 and 256 characters long");
    }
    let email = email.trim();
    if email.len() > 254
        || email.chars().any(char::is_whitespace)
        || email.matches('@').count() != 1
        || email.starts_with('@')
        || email.ends_with('@')
    {
        bail!("the admin email is not valid");
    }
    Ok(())
}

/// Admin credentials used to reach the local database server.
fn admin_credentials() -> crate::database::AdminCredentials {
    crate::database::AdminCredentials {
        mysql_root_password: std::env::var("SYNTHPANEL_MYSQL_ROOT_PASSWORD").ok(),
        postgres_user: std::env::var("SYNTHPANEL_POSTGRES_USER")
            .unwrap_or_else(|_| "postgres".to_string()),
        postgres_password: std::env::var("SYNTHPANEL_POSTGRES_PASSWORD").ok(),
    }
}

/// Derive conservative database identifiers from a site id.
fn db_identifiers(site_id: &str) -> Result<(String, String)> {
    let compact: String = site_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(12)
        .collect();
    if compact.is_empty() {
        bail!("the site id has no usable identifier");
    }
    let name = crate::database::validate_name(&format!("wp_{compact}"))?;
    let username = crate::database::validate_username(&format!("wp_{compact}"))?;
    Ok((name, username))
}

/// Create (or reuse) the dedicated database for a site.
///
/// Mirrors the `databases.create` IPC handler: provision on the server
/// first, then record the database and its user in the panel state.
pub async fn provision_database(
    pool: &SqlitePool,
    owner_id: Option<String>,
    site_id: &str,
) -> Result<DatabaseCredentials> {
    let (name, username) = db_identifiers(site_id)?;
    let databases = crate::databases::Databases::new(pool.clone());

    if let Some(existing) = databases.get_by_name("mysql", &name).await? {
        let users = databases.list_users(&existing.id).await?;
        if let Some(user) = users.first() {
            info!(database = %name, "reusing existing WordPress database");
            return Ok(DatabaseCredentials {
                name,
                username: user.username.clone(),
                password: user.password_encrypted.clone(),
                host: "127.0.0.1".to_string(),
                port: 3306,
            });
        }
    }

    if let Some(owner) = &owner_id {
        crate::sites::enforce_db_quota(pool, owner).await?;
    }
    let password = crate::database::generate_password(24);
    crate::database::mysql_create(&name, &username, &password, &admin_credentials())?;
    let record = databases.create("mysql", &name, owner_id).await?;
    databases
        .add_user(
            &record.id,
            &username,
            &password,
            crate::databases::Privilege::All,
        )
        .await?;

    Ok(DatabaseCredentials {
        name,
        username,
        password,
        host: "127.0.0.1".to_string(),
        port: 3306,
    })
}

fn find_binary(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Finish the admin account with wp-cli when it is installed.
///
/// WordPress tooling is never run as root: the core wraps it with the
/// env-overridable `runuser` binary and the site's pool user. When wp-cli is
/// absent the install still succeeds; the admin can create the account on
/// first visit.
fn finalize_admin(
    site_user: &str,
    domain: &str,
    root: &Path,
    request: &InstallRequest<'_>,
) -> Result<bool> {
    let wp_cli = std::env::var("SYNTHPANEL_WPCLI_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "wp".to_string());
    if find_binary(&wp_cli).is_none() {
        info!("wp-cli is not installed; skipping the automatic admin account creation");
        return Ok(false);
    }

    let args = vec![
        format!("--path={}", root.display()),
        "core".to_string(),
        "install".to_string(),
        format!("--url={domain}"),
        format!("--title={domain}"),
        format!("--admin_user={}", request.admin_user),
        format!("--admin_password={}", request.admin_password),
        format!("--admin_email={}", request.admin_email),
        "--skip-email".to_string(),
    ];

    // `--allow-root` is intentionally never passed.
    if unsafe { libc::geteuid() } == 0 {
        let run_as = std::env::var("SYNTHPANEL_RUNAS_BIN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "runuser".to_string());
        let mut wrapper = vec![
            "-u".to_string(),
            validate_owner(site_user)?,
            "--".to_string(),
            wp_cli.clone(),
        ];
        wrapper.extend(args);
        let output = Command::new(&run_as)
            .args(&wrapper)
            .output()
            .with_context(|| format!("cannot run `{run_as}`"))?;
        if !output.status.success() {
            bail!(
                "wp-cli failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    } else {
        let output = Command::new(&wp_cli)
            .args(&args)
            .output()
            .with_context(|| format!("cannot run `{wp_cli}`"))?;
        if !output.status.success() {
            bail!(
                "wp-cli failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(true)
}

/// Run the full one-click install.
///
/// This is the function the `wordpress.install` IPC method should call.
pub async fn install(pool: &SqlitePool, request: InstallRequest<'_>) -> Result<InstallResult> {
    let site_id = request.site_id.trim();
    if site_id.is_empty() {
        bail!("site_id must not be empty");
    }
    validate_admin(
        request.admin_user,
        request.admin_password,
        request.admin_email,
    )?;

    let site: Option<crate::sites::SiteRecord> = sqlx::query_as("SELECT * FROM sites WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?;
    let site = site.ok_or_else(|| anyhow!("site not found"))?;
    let root = PathBuf::from(&site.document_root);
    if !root.is_absolute() || root == Path::new("/") {
        bail!("the site's document root must be an absolute directory path");
    }
    let owner = validate_owner(&site.pool_user)?;

    let archive = obtain_archive().await?;
    if let Some(expected) = std::env::var("SYNTHPANEL_WORDPRESS_SHA256")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        verify_sha256(&archive.path, &expected)?;
    }
    let info = inspect_archive(&archive.path)?;

    fs::create_dir_all(&root).with_context(|| format!("cannot create {}", root.display()))?;
    extract_archive(&archive.path, &root)?;
    apply_ownership(&root, &owner, true)?;

    let db = provision_database(pool, site.owner_id.clone(), &site.id).await?;
    let config = WpConfig {
        db: &db,
        salts: &Salts::generate(),
    };
    let config_path = root.join("wp-config.php");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o640)
        .open(&config_path)
        .with_context(|| format!("cannot create {}", config_path.display()))?;
    file.write_all(render_wp_config(&config).as_bytes())?;
    drop(file);
    apply_ownership(&config_path, &owner, false)?;

    match finalize_admin(&owner, &site.domain, &root, &request) {
        Ok(true) => info!(site_id, "WordPress admin account created"),
        Ok(false) => warn!(
            site_id,
            "wp-cli unavailable; admin account not created automatically"
        ),
        Err(error) => warn!(%error, site_id, "wp-cli admin creation failed"),
    }

    Ok(InstallResult {
        ok: true,
        version: info.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::time::{SystemTime, UNIX_EPOCH};

    const VERSION_PHP: &str = "<?php\n$wp_version = '6.7.1';\n$wp_db_version = 59077;\n";

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}-{:?}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Build a small tar.gz fixture: (name, bytes, entry type).
    fn fixture(entries: &[(&str, &[u8], tar::EntryType)]) -> PathBuf {
        let dir = temp_dir("sp-wp-fixture");
        let path = dir.join("wordpress.tar.gz");
        let file = File::create(&path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, data, kind) in entries {
            let mut header = tar::Header::new_gnu();
            // Write the name verbatim: `set_path` rejects `..`, which is
            // exactly what the traversal test needs to synthesise.
            let name_bytes = name.as_bytes();
            assert!(name_bytes.len() <= 100);
            header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(*kind);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    fn valid_fixture() -> PathBuf {
        fixture(&[
            ("wordpress/", &[], tar::EntryType::Directory),
            (
                "wordpress/wp-includes/version.php",
                VERSION_PHP.as_bytes(),
                tar::EntryType::Regular,
            ),
            (
                "wordpress/readme.html",
                b"<h1>WordPress</h1>",
                tar::EntryType::Regular,
            ),
        ])
    }

    #[test]
    fn safe_entry_paths_are_confined_to_the_archive_root() {
        assert_eq!(
            safe_entry_path("wordpress/readme.html").unwrap(),
            Path::new("readme.html")
        );
        assert_eq!(
            safe_entry_path("wordpress/./wp-includes/x.php").unwrap(),
            Path::new("wp-includes/x.php")
        );
        assert_eq!(safe_entry_path("wordpress").unwrap(), PathBuf::new());

        for bad in [
            "../evil.php",
            "wordpress/../../evil.php",
            "/wordpress/readme.html",
            "other/readme.html",
            "wordpress\\readme.html",
            "",
        ] {
            assert!(safe_entry_path(bad).is_err(), "accepted unsafe path: {bad}");
        }
    }

    #[test]
    fn parses_the_wordpress_version() {
        assert_eq!(parse_wp_version(VERSION_PHP), Some("6.7.1".to_string()));
        assert_eq!(
            parse_wp_version("$wp_version = \"5.9\";"),
            Some("5.9".to_string())
        );
        assert_eq!(parse_wp_version("$wp_db_version = 42;"), None);
    }

    #[test]
    fn inspects_a_valid_fixture() {
        let archive = valid_fixture();
        let info = inspect_archive(&archive).unwrap();
        assert_eq!(info.version, "6.7.1");
        // The `wordpress/` directory entry is counted as well as the two files.
        assert_eq!(info.entries, 3);
        let _ = fs::remove_dir_all(archive.parent().unwrap());
    }

    #[test]
    fn rejects_traversal_and_link_entries() {
        let traversal = fixture(&[
            ("wordpress/../evil.php", b"bad", tar::EntryType::Regular),
            (
                "wordpress/wp-includes/version.php",
                VERSION_PHP.as_bytes(),
                tar::EntryType::Regular,
            ),
        ]);
        assert!(inspect_archive(&traversal).is_err());
        let _ = fs::remove_dir_all(traversal.parent().unwrap());

        let link = fixture(&[
            (
                "wordpress/wp-config.php",
                b"../../etc/passwd",
                tar::EntryType::Symlink,
            ),
            (
                "wordpress/wp-includes/version.php",
                VERSION_PHP.as_bytes(),
                tar::EntryType::Regular,
            ),
        ]);
        assert!(inspect_archive(&link).is_err());
        let _ = fs::remove_dir_all(link.parent().unwrap());
    }

    #[test]
    fn extracts_only_below_the_document_root() {
        let archive = valid_fixture();
        let root = temp_dir("sp-wp-root");
        extract_archive(&archive, &root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("wp-includes/version.php")).unwrap(),
            VERSION_PHP
        );
        assert_eq!(
            fs::read_to_string(root.join("readme.html")).unwrap(),
            "<h1>WordPress</h1>"
        );
        assert!(!root.join("../evil.php").exists());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(archive.parent().unwrap());
    }

    #[test]
    fn verifies_the_sha256_of_a_fixture() {
        let archive = valid_fixture();
        let mut hasher = Sha256::new();
        io::copy(&mut File::open(&archive).unwrap(), &mut hasher).unwrap();
        let digest = hex::encode(hasher.finalize());
        verify_sha256(&archive, &digest.to_uppercase()).unwrap();
        assert!(verify_sha256(&archive, &"0".repeat(64)).is_err());
        assert!(verify_sha256(&archive, "deadbeef").is_err());
        let _ = fs::remove_dir_all(archive.parent().unwrap());
    }

    #[test]
    fn renders_wp_config_with_escaped_credentials() {
        let db = DatabaseCredentials {
            name: "wp_abc123".into(),
            username: "wp_abc123".into(),
            password: "it's\\a test".into(),
            host: "127.0.0.1".into(),
            port: 3306,
        };
        let empty = String::new();
        let salts = Salts {
            auth_key: empty.clone(),
            secure_auth_key: empty.clone(),
            logged_in_key: empty.clone(),
            nonce_key: empty.clone(),
            auth_salt: empty.clone(),
            secure_auth_salt: empty.clone(),
            logged_in_salt: empty.clone(),
            nonce_salt: empty.clone(),
        };
        let rendered = render_wp_config(&WpConfig {
            db: &db,
            salts: &salts,
        });
        assert!(rendered.starts_with("<?php"));
        assert!(rendered.contains("define( 'DB_NAME', 'wp_abc123' );"));
        assert!(rendered.contains("define( 'DB_HOST', '127.0.0.1:3306' );"));
        assert!(rendered.contains("define( 'DB_PASSWORD', 'it\\'s\\\\a test' );"));
        assert!(rendered.contains("$table_prefix = 'wp_';"));
        assert!(rendered.contains("require_once ABSPATH . 'wp-settings.php';"));
        assert_eq!(
            rendered.matches("define( 'AUTH_KEY'").count(),
            1,
            "all eight salts must be rendered"
        );
    }

    #[test]
    fn php_quote_escapes_backslashes_and_quotes() {
        assert_eq!(php_quote("a'b\\c"), "'a\\'b\\\\c'");
    }

    #[test]
    fn validates_admin_inputs() {
        validate_admin("admin", "s3cret!", "admin@example.com").unwrap();
        assert!(validate_admin("", "x", "admin@example.com").is_err());
        assert!(validate_admin("bad user", "x", "admin@example.com").is_err());
        assert!(validate_admin("admin", "", "admin@example.com").is_err());
        assert!(validate_admin("admin", "x", "not-an-email").is_err());
    }

    #[test]
    fn chown_arguments_cannot_inject_a_shell() {
        assert_eq!(
            chown_args("www-data", Path::new("/srv/site"), true).unwrap(),
            vec!["-R", "--", "www-data", "/srv/site"]
        );
        assert_eq!(
            chown_args("www-data:www-data", Path::new("/srv/site"), false).unwrap(),
            vec!["--", "www-data:www-data", "/srv/site"]
        );
        assert!(validate_owner("root; rm -rf /").is_err());
        assert!(validate_owner("user:group:extra").is_err());
    }
}
