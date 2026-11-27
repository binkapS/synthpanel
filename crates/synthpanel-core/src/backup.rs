//! Backup and restore engine.
//!
//! A backup is a single `.tar.gz` archive that contains the panel-managed
//! configuration (nginx vhosts, PHP-FPM pools), the site document roots and
//! — optionally — a database dump. The archive can optionally be encrypted
//! with a passphrase and is then uploaded to a destination.

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tracing::{info, warn};

/// The name of the manifest stored inside every archive.
pub const MANIFEST_NAME: &str = "synthpanel-manifest.json";

/// The magic bytes of an encrypted archive.
const ENCRYPTED_MAGIC: &[u8] = b"SPAE1";

/// What a backup contains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contents {
    pub vhosts: bool,
    pub pools: bool,
    pub files: bool,
    pub databases: bool,
}

impl Default for Contents {
    fn default() -> Self {
        Self {
            vhosts: true,
            pools: true,
            files: true,
            databases: false,
        }
    }
}

/// A validated set of glob patterns that are skipped while a backup walks a
/// site's document root. Patterns are relative to the site root and use `/` as
/// the separator; `*` and `?` match inside one path component and `**` spans
/// whole components. A pattern with a single component matches that name at
/// any depth, so `cache` skips every directory called `cache`.
///
/// An empty set is the historical behaviour: every file is archived.
#[derive(Debug, Clone, Default)]
pub struct Excludes {
    patterns: Vec<ExcludePattern>,
}

#[derive(Debug, Clone)]
struct ExcludePattern {
    segments: Vec<String>,
}

impl Excludes {
    /// Validate and compile the patterns supplied by an operator.
    pub fn new(patterns: &[String]) -> Result<Self> {
        validate_exclude_patterns(patterns)?;
        Ok(Self {
            patterns: patterns.iter().map(|p| ExcludePattern::new(p)).collect(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether a path relative to a site root is excluded.
    pub fn is_excluded(&self, relative: &Path) -> bool {
        self.matches_any(&[relative])
    }

    /// A path may be described both relative to a site root and relative to
    /// the `www` directory; accept a match against either spelling.
    fn matches_any(&self, candidates: &[&Path]) -> bool {
        self.patterns
            .iter()
            .any(|pattern| candidates.iter().any(|path| pattern.matches(path)))
    }
}

impl ExcludePattern {
    fn new(pattern: &str) -> Self {
        Self {
            segments: pattern_segments(pattern),
        }
    }

    fn matches(&self, path: &Path) -> bool {
        let segments = path_segments(path);
        if segments.is_empty() {
            return false;
        }
        if self.segments.len() == 1 {
            return segments
                .iter()
                .any(|name| segment_matches(&self.segments[0], name));
        }
        match_segments(&self.segments, &segments)
    }
}

/// Split a validated pattern into comparable path components.
fn pattern_segments(pattern: &str) -> Vec<String> {
    pattern
        .trim()
        .trim_end_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect()
}

fn path_segments(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn match_segments(pattern: &[String], path: &[String]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(part) if part == "**" => {
            match_segments(&pattern[1..], path)
                || (!path.is_empty() && match_segments(pattern, &path[1..]))
        }
        Some(part) => {
            !path.is_empty()
                && segment_matches(part, &path[0])
                && match_segments(&pattern[1..], &path[1..])
        }
    }
}

/// Shell-style wildcard matching inside a single path component.
fn segment_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut retry = 0usize;
    while ni < name.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            pi += 1;
            retry = ni;
        } else if let Some(index) = star {
            pi = index + 1;
            retry += 1;
            ni = retry;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// Reject patterns that could reach outside a document root or that cannot be
/// interpreted as a relative path. Validation happens both when patterns are
/// stored and again when they are used for a backup.
pub fn validate_exclude_patterns(patterns: &[String]) -> Result<()> {
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            bail!("an exclude pattern may not be empty");
        }
        if trimmed.starts_with('/') {
            bail!("exclude patterns must be relative: `{pattern}`");
        }
        if trimmed.contains('\\') || trimmed.contains('\0') {
            bail!("exclude patterns must use '/' and contain no NUL: `{pattern}`");
        }
        let segments: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
        if segments.is_empty() {
            bail!("an exclude pattern needs a path component: `{pattern}`");
        }
        if segments.iter().any(|part| *part == ".." || *part == ".") {
            bail!("exclude patterns may not traverse paths: `{pattern}`");
        }
    }
    Ok(())
}

/// Row key of the global exclude list. The schema uses `''` because an empty
/// string can never be a generated site id, while `NULL` could not take part
/// in a single-column primary key alongside per-site rows.
pub const GLOBAL_EXCLUDES_KEY: &str = "";

/// Store the exclude patterns of a site, or of every site when `site_id` is
/// omitted. Passing an empty list clears the stored patterns.
pub async fn set_excludes(
    pool: &SqlitePool,
    site_id: Option<&str>,
    patterns: &[String],
) -> Result<()> {
    validate_exclude_patterns(patterns)?;
    let key = site_id.unwrap_or(GLOBAL_EXCLUDES_KEY);
    let json = serde_json::to_string(patterns)?;
    sqlx::query(
        "INSERT INTO backup_excludes (site_id, patterns, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(site_id) DO UPDATE SET
           patterns = excluded.patterns,
           updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(json)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Read one scope's patterns from the database.
pub async fn load_excludes(pool: &SqlitePool, site_id: Option<&str>) -> Result<Vec<String>> {
    let key = site_id.unwrap_or(GLOBAL_EXCLUDES_KEY);
    let json: Option<String> =
        sqlx::query_scalar("SELECT patterns FROM backup_excludes WHERE site_id = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    match json {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}

/// Global patterns followed by a site's own patterns. A full backup passes
/// `None` and therefore receives the global list only.
pub async fn effective_excludes(pool: &SqlitePool, site_id: Option<&str>) -> Result<Vec<String>> {
    let mut patterns = load_excludes(pool, None).await?;
    if let Some(site_id) = site_id {
        patterns.extend(load_excludes(pool, Some(site_id)).await?);
    }
    Ok(patterns)
}

/// The kind of backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Everything the panel manages.
    Full,
    /// A single site.
    Site,
}

/// The manifest stored inside an archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub created_at: String,
    pub kind: String,
    pub site_id: Option<String>,
    pub site_domain: Option<String>,
    /// The document root of a site backup, when one was supplied.
    pub site_root: Option<PathBuf>,
    pub contents: Contents,
    pub entries: Vec<String>,
    pub checksum: String,
}

/// The result of a backup run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupResult {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub duration_ms: u128,
    pub entries: Vec<String>,
    pub location: String,
    pub checksum: String,
    pub encrypted: bool,
}

/// The result of a restore run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResult {
    pub id: String,
    pub restored: Vec<String>,
    pub duration_ms: u128,
}

/// Where an archive is stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Destination {
    /// A directory on the local disk.
    Local { path: PathBuf },
    /// An SFTP server; the `sftp` CLI is used for the transfer.
    Sftp {
        host: String,
        port: u16,
        user: String,
        key_path: Option<PathBuf>,
        path: String,
    },
    /// An S3-compatible bucket.
    S3 {
        endpoint: String,
        region: String,
        bucket: String,
        prefix: Option<String>,
        access_key: String,
        secret_key: String,
    },
    /// Any remote supported by the `rclone` CLI (Google Drive, S3, SFTP, ...).
    /// `remote` is the configured remote name and `path` is the directory
    /// inside it; both are passed straight through as rclone arguments.
    Rclone {
        #[serde(alias = "remote_name")]
        remote: String,
        path: String,
    },
}

impl Destination {
    /// A short label for the UI.
    pub fn label(&self) -> String {
        match self {
            Destination::Local { path } => format!("local:{}", path.display()),
            Destination::Sftp { host, path, .. } => format!("sftp://{host}{path}"),
            Destination::S3 {
                endpoint, bucket, ..
            } => format!("s3://{bucket} @ {endpoint}"),
            Destination::Rclone { remote, path } => format!("rclone://{remote}/{path}"),
        }
    }

    /// Validate the destination before it is used.
    pub fn validate(&self) -> Result<()> {
        match self {
            Destination::Local { path } => {
                if path.as_os_str().is_empty() {
                    bail!("the local destination needs a path");
                }
            }
            Destination::Sftp {
                host, user, path, ..
            } => {
                if host.trim().is_empty() {
                    bail!("the SFTP destination needs a host");
                }
                if user.trim().is_empty() {
                    bail!("the SFTP destination needs a user");
                }
                if path.trim().is_empty() {
                    bail!("the SFTP destination needs a remote path");
                }
            }
            Destination::S3 {
                endpoint,
                bucket,
                access_key,
                secret_key,
                ..
            } => {
                if endpoint.trim().is_empty() {
                    bail!("the S3 destination needs an endpoint");
                }
                if bucket.trim().is_empty() {
                    bail!("the S3 destination needs a bucket");
                }
                if access_key.trim().is_empty() || secret_key.trim().is_empty() {
                    bail!("the S3 destination needs an access key and a secret");
                }
                check_s3_endpoint(endpoint)?;
            }
            Destination::Rclone { remote, path } => {
                check_rclone_remote(remote, path)?;
            }
        }
        Ok(())
    }
}

/// Check the remote name and path before they reach a child process.
///
/// Arguments are passed to `rclone` as separate argv entries, so there is no
/// shell interpolation, but a leading dash could still be read as an option
/// and a `:` would let a caller select a different remote than the one named.
pub fn check_rclone_remote(remote: &str, path: &str) -> Result<()> {
    let remote = remote.trim();
    if remote.is_empty() {
        bail!("the rclone destination needs a remote name");
    }
    if remote.starts_with('-')
        || remote.contains(':')
        || remote.contains('/')
        || remote.contains('\\')
        || remote.chars().any(char::is_whitespace)
    {
        bail!("the rclone remote name may not contain ':', '/', whitespace or a leading '-'");
    }
    if path.trim().is_empty() || path.contains('\0') || path.starts_with('-') {
        bail!("the rclone destination needs a remote path");
    }
    Ok(())
}

/// The rclone executable; deployments can point this at a wrapper or a
/// non-PATH installation.
pub fn rclone_bin() -> PathBuf {
    std::env::var_os("SYNTHPANEL_RCLONE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rclone"))
}

/// `remote:path` target understood by rclone.
pub fn rclone_target(remote: &str, path: &str) -> String {
    format!("{remote}:{}", path.trim_start_matches('/'))
}

/// The location recorded for an uploaded archive, parseable by
/// `rclone_remote_from_location` on restore.
pub fn rclone_location(remote: &str, path: &str, name: &str) -> String {
    let path = path.trim_matches('/');
    if path.is_empty() {
        format!("rclone://{remote}/{name}")
    } else {
        format!("rclone://{remote}/{path}/{name}")
    }
}

/// Turn a stored location back into the `remote:path/name` rclone target.
pub fn rclone_remote_from_location(location: &str) -> Result<String> {
    let value = location.strip_prefix("rclone://").unwrap_or(location);
    // Accept rclone's own `remote:path` spelling as well as the recorded
    // `rclone://remote/path` one.
    if let Some((remote, path)) = value.split_once(':') {
        if !remote.is_empty() && !remote.contains('/') && !path.is_empty() {
            check_rclone_remote(remote, path)?;
            return Ok(rclone_target(remote, path));
        }
    }
    if let Some((remote, path)) = value.split_once('/') {
        if remote.is_empty() || path.trim().is_empty() {
            bail!("invalid rclone archive location");
        }
        Ok(rclone_target(remote, path))
    } else {
        bail!("invalid rclone archive location")
    }
}

/// Run rclone with validated arguments. The binary is overridable so tests and
/// deployments can supply a wrapper without changing the system PATH.
fn run_rclone(args: &[&str]) -> Result<std::process::Output> {
    run_rclone_with(&rclone_bin(), args)
}

fn run_rclone_with(bin: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = std::process::Command::new(bin)
        .args(args)
        .output()
        .context("cannot run rclone")?;
    if !output.status.success() {
        bail!(
            "rclone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

/// Check that an S3 endpoint is one the panel may talk to.
///
/// The core runs as root, so an endpoint from a request would turn the panel
/// into a proxy for every address the server can reach — the cloud metadata
/// service, an internal service, a host the operator trusts. The endpoint
/// therefore has to be a public HTTPS host.
pub fn check_s3_endpoint(endpoint: &str) -> Result<()> {
    let endpoint = endpoint.trim();
    if !endpoint.starts_with("https://") {
        bail!("the S3 endpoint must use https");
    }
    let rest = &endpoint["https://".len()..];
    let host = rest
        .split('/')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        bail!("the S3 endpoint needs a host");
    }
    // The addresses that must never be reached from a root process.
    let blocked = [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "::1",
        "169.254.169.254",
        "metadata.google.internal",
    ];
    if blocked.contains(&host.as_str()) || host.ends_with(".localhost") {
        bail!("the S3 endpoint may not point at a local address");
    }
    if host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("127.")
        || host.starts_with("169.254.")
    {
        bail!("the S3 endpoint may not point at a private address");
    }
    Ok(())
}

/// The inputs of a backup run.
#[derive(Debug, Clone)]
pub struct BackupPlan {
    pub id: String,
    pub kind: Kind,
    pub site_id: Option<String>,
    pub site_domain: Option<String>,
    /// The document root of a site backup, when one was supplied.
    pub site_root: Option<PathBuf>,
    pub contents: Contents,
    pub destination: Destination,
    pub passphrase: Option<String>,
    /// Where the staging directory lives.
    pub staging_dir: PathBuf,
    /// Directory holding the generated nginx vhosts.
    pub vhost_dir: PathBuf,
    /// Directory holding the generated PHP-FPM pools.
    pub pool_dir: PathBuf,
    /// Directory holding the site document roots.
    pub www_dir: PathBuf,
}

/// Build the archive name for a backup.
pub fn archive_name(id: &str, created_at: &str) -> String {
    let stamp = created_at
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    format!("synthpanel-{id}-{stamp}.tar.gz")
}

/// Create a backup and upload it to the destination, with no excludes.
pub async fn create_backup(plan: &BackupPlan) -> Result<BackupResult> {
    create_backup_with_excludes(plan, &[]).await
}

/// Create a backup, skipping the site files matched by `patterns`.
///
/// This is the entry point for callers that load stored global/per-site
/// excludes; an empty list keeps the historical, unfiltered behaviour.
pub async fn create_backup_with_excludes(
    plan: &BackupPlan,
    patterns: &[String],
) -> Result<BackupResult> {
    let excludes = Excludes::new(patterns)?;
    plan.destination.validate()?;
    let started = std::time::Instant::now();

    let work_dir = plan.staging_dir.join(format!("build-{}", plan.id));
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("cannot create {}", work_dir.display()))?;

    let mut entries: Vec<String> = Vec::new();

    // 1. Configuration
    if plan.contents.vhosts {
        copy_tree(&plan.vhost_dir, &work_dir.join("nginx"))?;
    }
    if plan.contents.pools {
        copy_tree(&plan.pool_dir, &work_dir.join("php"))?;
    }

    // 2. Site document roots
    if plan.contents.files {
        if let Some(root) = &plan.site_root {
            if root.exists() {
                let name = root
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| "site".to_string());
                copy_tree_excluding(
                    root,
                    &work_dir.join("www").join(&name),
                    &excludes,
                    Path::new(&name),
                )?;
            }
        } else {
            match &plan.site_id {
                Some(site_id) => {
                    let source = plan.www_dir.join(site_id);
                    if source.exists() {
                        copy_tree_excluding(
                            &source,
                            &work_dir.join("www").join(site_id),
                            &excludes,
                            Path::new(site_id),
                        )?;
                    }
                }
                None => {
                    // A full backup walks every site root under `www`. Each
                    // site is filtered relative to its own root, so a pattern
                    // such as `cache` has the same meaning for full and for
                    // per-site backups.
                    copy_www_directory(&plan.www_dir, &work_dir.join("www"), &excludes)?;
                }
            }
        }
    }

    // 3. Database dumps
    if plan.contents.databases {
        dump_databases(plan, &work_dir.join("databases"), &mut entries)?;
    }

    // The manifest lists exactly what the archive contains, relative to the
    // archive root.
    let mut staged: Vec<PathBuf> = Vec::new();
    collect_files(&work_dir, &mut staged)?;
    entries = staged
        .iter()
        .filter_map(|path| {
            path.strip_prefix(&work_dir)
                .ok()
                .map(|relative| relative.to_string_lossy().to_string())
        })
        .collect();
    entries.sort();

    // 4. Manifest
    let manifest = Manifest {
        version: 1,
        created_at: Utc::now().to_rfc3339(),
        kind: match plan.kind {
            Kind::Full => "full".into(),
            Kind::Site => "site".into(),
        },
        site_id: plan.site_id.clone(),
        site_domain: plan.site_domain.clone(),
        site_root: plan.site_root.clone(),
        contents: plan.contents.clone(),
        entries: entries.clone(),
        checksum: String::new(),
    };
    let manifest_path = work_dir.join(MANIFEST_NAME);
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(&manifest_path, &manifest_json)?;

    // 5. Archive
    let archive_name = archive_name(&plan.id, &manifest.created_at);
    let archive_path = plan.staging_dir.join(&archive_name);
    let checksum = create_archive(&work_dir, &archive_path)?;

    // 6. Optional encryption
    let encrypted = plan.passphrase.is_some();
    let final_path = if let Some(passphrase) = &plan.passphrase {
        let encrypted_path = plan.staging_dir.join(format!("{archive_name}.enc"));
        encrypt_file(&archive_path, &encrypted_path, passphrase)?;
        let _ = std::fs::remove_file(&archive_path);
        encrypted_path
    } else {
        archive_path
    };

    let size = std::fs::metadata(&final_path)?.len();

    // 7. Upload
    let location = upload(&plan.destination, &final_path, &archive_name).await?;
    let _ = std::fs::remove_file(&final_path);
    let _ = std::fs::remove_dir_all(&work_dir);

    info!(id = %plan.id, size, location = %location, "backup created");
    Ok(BackupResult {
        id: plan.id.clone(),
        name: archive_name,
        size,
        duration_ms: started.elapsed().as_millis(),
        entries,
        location,
        checksum,
        encrypted,
    })
}

/// Copy a directory tree into the staging area (configuration files are never
/// filtered by backup excludes).
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    copy_tree_inner(
        source,
        destination,
        &Excludes::default(),
        Path::new(""),
        Path::new(""),
    )
}

/// Copy a site tree, skipping excluded paths. `prefix` is the path of the
/// site root relative to `www`, used to also match site-qualified patterns.
fn copy_tree_excluding(
    source: &Path,
    destination: &Path,
    excludes: &Excludes,
    prefix: &Path,
) -> Result<()> {
    copy_tree_inner(source, destination, excludes, Path::new(""), prefix)
}

fn copy_tree_inner(
    source: &Path,
    destination: &Path,
    excludes: &Excludes,
    relative: &Path,
    prefix: &Path,
) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let child = relative.join(&name);
        let qualified = prefix.join(&child);
        if excludes.matches_any(&[&child, &qualified]) {
            continue;
        }
        let target = destination.join(&name);
        if path.is_dir() {
            copy_tree_inner(&path, &target, excludes, &child, prefix)?;
        } else {
            std::fs::copy(&path, &target).with_context(|| {
                format!("cannot copy {} to {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

/// Walk the `www` directory of a full backup, applying excludes inside each
/// site root while leaving its overall layout untouched.
fn copy_www_directory(source: &Path, destination: &Path, excludes: &Excludes) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if excludes.is_excluded(&Path::new(&name)) {
            continue;
        }
        let target = destination.join(&name);
        if path.is_dir() {
            copy_tree_excluding(&path, &target, excludes, Path::new(&name))?;
        } else {
            std::fs::copy(&path, &target).with_context(|| {
                format!("cannot copy {} to {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

/// Dump the managed databases into the staging area.
fn dump_databases(plan: &BackupPlan, destination: &Path, entries: &mut Vec<String>) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let engine = std::env::var("SYNTHPANEL_BACKUP_DB_ENGINE").unwrap_or_else(|_| "mysql".into());
    let name = plan.site_id.as_deref().unwrap_or("all");
    match engine.as_str() {
        "postgres" => {
            let output = std::process::Command::new("pg_dumpall")
                .arg("--clean")
                .arg("--if-exists")
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    let path = destination.join(format!("{name}.sql"));
                    std::fs::write(&path, output.stdout)?;
                    entries.push(format!("databases/{name}.sql"));
                }
                Ok(output) => warn!(
                    stderr = %String::from_utf8_lossy(&output.stderr),
                    "pg_dumpall failed"
                ),
                Err(error) => warn!(%error, "cannot run pg_dumpall"),
            }
        }
        _ => {
            let output = std::process::Command::new("mysqldump")
                .arg("--all-databases")
                .arg("--single-transaction")
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    let path = destination.join(format!("{name}.sql"));
                    std::fs::write(&path, output.stdout)?;
                    entries.push(format!("databases/{name}.sql"));
                }
                Ok(output) => warn!(
                    stderr = %String::from_utf8_lossy(&output.stderr),
                    "mysqldump failed"
                ),
                Err(error) => warn!(%error, "cannot run mysqldump"),
            }
        }
    }
    Ok(())
}

/// Create a gzipped tar archive and return its SHA-256 checksum.
pub fn create_archive(source_dir: &Path, archive_path: &Path) -> Result<String> {
    let file = File::create(archive_path)
        .with_context(|| format!("cannot create {}", archive_path.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);

    // Deterministic ordering keeps the checksums stable.
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(source_dir, &mut files)?;
    files.sort();

    for path in &files {
        let relative = path
            .strip_prefix(source_dir)
            .expect("path is inside the source directory");
        builder.append_path_with_name(path, relative)?;
    }
    let encoder = builder.into_inner()?;
    let mut file = encoder.finish()?;
    file.flush()?;
    checksum_file(archive_path)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Compute the SHA-256 checksum of a file.
pub fn checksum_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Encrypt a file with a passphrase.
pub fn encrypt_file(source: &Path, destination: &Path, passphrase: &str) -> Result<()> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let key = derive_key(passphrase);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce_bytes = random_bytes(12);
    let nonce = Nonce::from_slice(nonce_bytes.as_slice());

    let plaintext = std::fs::read(source)?;
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| anyhow!("cannot encrypt the archive"))?;

    let mut file = File::create(destination)?;
    file.write_all(ENCRYPTED_MAGIC)?;
    file.write_all(nonce.as_slice())?;
    file.write_all(&ciphertext)?;
    file.flush()?;
    Ok(())
}

/// Decrypt a file with a passphrase.
pub fn decrypt_file(source: &Path, destination: &Path, passphrase: &str) -> Result<()> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let raw = std::fs::read(source)?;
    if raw.len() < ENCRYPTED_MAGIC.len() + 12 {
        bail!("the archive is truncated");
    }
    if &raw[..ENCRYPTED_MAGIC.len()] != ENCRYPTED_MAGIC {
        bail!("the archive is not encrypted");
    }
    let nonce = Nonce::from_slice(&raw[ENCRYPTED_MAGIC.len()..ENCRYPTED_MAGIC.len() + 12]);
    let ciphertext = &raw[ENCRYPTED_MAGIC.len() + 12..];

    let key = derive_key(passphrase);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("cannot decrypt the archive: wrong passphrase?"))?;
    std::fs::write(destination, plaintext)?;
    Ok(())
}

/// Derive an encryption key from a passphrase.
fn derive_key(passphrase: &str) -> [u8; 32] {
    // Argon2 is already a dependency of the panel; reuse it so a passphrase
    // is never used directly as key material.
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::encode_b64(b"synthpanel-backup").expect("static salt is valid");
    let hash = argon2::Argon2::default()
        .hash_password(passphrase.as_bytes(), &salt)
        .expect("argon2 hashing cannot fail");
    let bytes = hash.hash.expect("the hash has bytes");
    let mut key = [0u8; 32];
    let raw = bytes.as_ref();
    for (index, byte) in raw.iter().take(32).enumerate() {
        key[index] = *byte;
    }
    key
}

fn random_bytes(length: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; length];
    rand::thread_rng().fill_bytes(&mut buffer);
    buffer
}

/// Read the manifest out of an archive.
pub fn read_manifest(archive_path: &Path) -> Result<Manifest> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut manifest: Option<Manifest> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        if path.trim_start_matches("./") == MANIFEST_NAME {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            manifest = Some(serde_json::from_str(&contents)?);
            break;
        }
    }
    manifest.ok_or_else(|| anyhow!("the archive has no manifest"))
}

/// List the files inside an archive.
pub fn list_archive(archive_path: &Path) -> Result<Vec<String>> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut names = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        names.push(entry.path()?.to_string_lossy().to_string());
    }
    names.sort();
    Ok(names)
}

/// Verify that an archive is readable and carries a manifest.
pub fn verify_archive(archive_path: &Path) -> Result<Manifest> {
    let manifest = read_manifest(archive_path)?;
    let names = list_archive(archive_path)?;
    for entry in &manifest.entries {
        let normalised = entry.trim_start_matches("./");
        if !names
            .iter()
            .any(|name| name.trim_start_matches("./") == normalised)
        {
            bail!("the archive is missing `{entry}`");
        }
    }
    Ok(manifest)
}

/// Restore an archive into the target directories.
pub fn restore_archive(
    archive_path: &Path,
    vhost_dir: &Path,
    pool_dir: &Path,
    www_dir: &Path,
) -> Result<RestoreResult> {
    let started = std::time::Instant::now();
    let manifest = read_manifest(archive_path)?;
    let staging = archive_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("restore-{}", manifest.created_at.replace(':', "-")));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    {
        let file = File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(&staging)?;
    }

    let mut restored = Vec::new();
    restore_component(&staging.join("nginx"), vhost_dir, &mut restored)?;
    restore_component(&staging.join("php"), pool_dir, &mut restored)?;
    // A site backup records its original document root; restore the tree next
    // to it so a custom root keeps its place.
    let www_destination = match &manifest.site_root {
        Some(root) => root.parent().unwrap_or(www_dir),
        None => www_dir,
    };
    restore_component(&staging.join("www"), www_destination, &mut restored)?;

    let _ = std::fs::remove_dir_all(&staging);
    info!(entries = restored.len(), "archive restored");
    Ok(RestoreResult {
        id: manifest
            .site_id
            .clone()
            .unwrap_or_else(|| "backup".to_string()),
        restored,
        duration_ms: started.elapsed().as_millis(),
    })
}

fn restore_component(source: &Path, destination: &Path, restored: &mut Vec<String>) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let target = destination.join(&name);
        if path.is_dir() {
            restore_component(&path, &target, restored)?;
        } else {
            std::fs::copy(&path, &target).with_context(|| {
                format!("cannot restore {} to {}", path.display(), target.display())
            })?;
            restored.push(target.to_string_lossy().to_string());
        }
    }
    Ok(())
}

/// Upload an archive to the destination.
pub async fn upload(destination: &Destination, local: &Path, name: &str) -> Result<String> {
    match destination {
        Destination::Local { path } => {
            std::fs::create_dir_all(path)?;
            let target = path.join(name);
            std::fs::copy(local, &target)
                .with_context(|| format!("cannot copy the archive to {}", target.display()))?;
            Ok(target.to_string_lossy().to_string())
        }
        Destination::Sftp {
            host,
            port,
            user,
            key_path,
            path,
        } => {
            let target = format!("{}/{name}", path.trim_end_matches('/'));
            let key = sftp_key_path(user).or_else(|| key_path.clone());
            upload_sftp(host, *port, user, key.as_deref(), local, &target)?;
            Ok(format!("sftp://{host}{target}"))
        }
        Destination::S3 {
            endpoint,
            region,
            bucket,
            prefix,
            access_key,
            secret_key,
        } => {
            let key = match prefix {
                Some(prefix) => format!("{}/{name}", prefix.trim_end_matches('/')),
                None => name.to_string(),
            };
            upload_s3(
                endpoint, region, bucket, &key, access_key, secret_key, local,
            )
            .await?;
            Ok(format!("s3://{bucket}/{key}"))
        }
        Destination::Rclone { remote, path } => {
            let source = local.to_string_lossy().into_owned();
            let target = format!("{}/{name}", rclone_target(remote, path));
            run_rclone(&["copyto", &source, &target])?;
            Ok(rclone_location(remote, path, name))
        }
    }
}

/// Download an archive from the destination.
pub async fn download(destination: &Destination, remote: &str, local: &Path) -> Result<()> {
    match destination {
        Destination::Local { path } => {
            let source = path.join(
                remote
                    .rsplit('/')
                    .next()
                    .ok_or_else(|| anyhow!("invalid archive name"))?,
            );
            std::fs::copy(&source, local)
                .with_context(|| format!("cannot copy {}", source.display()))?;
            Ok(())
        }
        Destination::Sftp {
            host,
            port,
            user,
            key_path,
            ..
        } => {
            let key = sftp_key_path(user).or_else(|| key_path.clone());
            download_sftp(host, *port, user, key.as_deref(), remote, local)?;
            Ok(())
        }
        Destination::S3 {
            endpoint,
            region,
            bucket,
            access_key,
            secret_key,
            ..
        } => {
            let key = remote
                .strip_prefix(&format!("s3://{bucket}/"))
                .unwrap_or(remote);
            download_s3(endpoint, region, bucket, key, access_key, secret_key, local).await?;
            Ok(())
        }
        Destination::Rclone { .. } => {
            let source = rclone_remote_from_location(remote)?;
            let target = local.to_string_lossy().into_owned();
            run_rclone(&["copyto", &source, &target])?;
            Ok(())
        }
    }
}

/// List the archives stored at the destination.
pub async fn list(destination: &Destination) -> Result<Vec<String>> {
    match destination {
        Destination::Local { path } => {
            let mut names = Vec::new();
            if path.exists() {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        names.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
            names.sort();
            Ok(names)
        }
        Destination::Sftp { .. } => Ok(Vec::new()),
        Destination::S3 { .. } => Ok(Vec::new()),
        Destination::Rclone { remote, path } => {
            let target = rclone_target(remote, path);
            match run_rclone(&["lsf", "--files-only", &target]) {
                Ok(output) => {
                    let mut names: Vec<String> = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(String::from)
                        .collect();
                    names.sort();
                    Ok(names)
                }
                Err(error) => {
                    warn!(%error, "cannot list the rclone remote");
                    Ok(Vec::new())
                }
            }
        }
    }
}

/// Transfer a file with the `sftp` CLI.
fn upload_sftp(
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<&Path>,
    local: &Path,
    remote: &str,
) -> Result<()> {
    let batch = format!("put {} {}\n", local.display(), remote);
    run_sftp(host, port, user, key_path, &batch)
}

/// Fetch a file with the `sftp` CLI.
fn download_sftp(
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<&Path>,
    remote: &str,
    local: &Path,
) -> Result<()> {
    let batch = format!("get {} {}\n", remote, local.display());
    run_sftp(host, port, user, key_path, &batch)
}

/// The private key an SFTP destination may use.
///
/// The key is never taken from a request: the core would offer an arbitrary
/// file to the SSH client, and the file manager can write files.
pub fn sftp_key_path(user: &str) -> Option<PathBuf> {
    let base = std::env::var_os("SYNTHPANEL_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/synthpanel"));
    let key = base.join("sftp-keys").join(format!("{user}.key"));
    key.is_file().then_some(key)
}

fn run_sftp(host: &str, port: u16, user: &str, key_path: Option<&Path>, batch: &str) -> Result<()> {
    let mut command = std::process::Command::new("sftp");
    command.arg("-o").arg("BatchMode=yes");
    command.arg("-P").arg(port.to_string());
    if let Some(key) = key_path {
        command.arg("-i").arg(key);
    }
    command.arg(format!("{user}@{host}"));
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command.spawn().context("cannot run sftp")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(batch.as_bytes())?;
        drop(stdin);
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "sftp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Upload a file to an S3-compatible bucket using SigV4.
async fn upload_s3(
    endpoint: &str,
    region: &str,
    bucket: &str,
    key: &str,
    access_key: &str,
    secret_key: &str,
    local: &Path,
) -> Result<()> {
    let body = std::fs::read(local)?;
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);
    let client = reqwest::Client::new();
    let response = s3_request(
        &client,
        S3Request {
            method: "PUT",
            url: &url,
            region,
            access_key,
            secret_key,
            body: &body,
            content_type: "application/octet-stream",
        },
    )
    .await?;
    if !response.status().is_success() {
        bail!("s3 upload failed with status {}", response.status());
    }
    Ok(())
}

/// Download a file from an S3-compatible bucket.
async fn download_s3(
    endpoint: &str,
    region: &str,
    bucket: &str,
    key: &str,
    access_key: &str,
    secret_key: &str,
    local: &Path,
) -> Result<()> {
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);
    let client = reqwest::Client::new();
    let response = s3_request(
        &client,
        S3Request {
            method: "GET",
            url: &url,
            region,
            access_key,
            secret_key,
            body: &[],
            content_type: "",
        },
    )
    .await?;
    if !response.status().is_success() {
        bail!("s3 download failed with status {}", response.status());
    }
    let bytes = response.bytes().await?;
    std::fs::write(local, &bytes)?;
    Ok(())
}

/// The inputs of a signed S3 request.
struct S3Request<'a> {
    method: &'a str,
    url: &'a str,
    region: &'a str,
    access_key: &'a str,
    secret_key: &'a str,
    body: &'a [u8],
    content_type: &'a str,
}

/// Sign and send an S3 request with AWS Signature Version 4.
async fn s3_request(client: &reqwest::Client, request: S3Request<'_>) -> Result<reqwest::Response> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let S3Request {
        method,
        url,
        region,
        access_key,
        secret_key,
        body,
        content_type,
    } = request;

    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow!("the S3 endpoint must include a scheme"))?;
    let _ = scheme;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let host = authority.to_string();
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };

    let payload_hash = hex::encode(Sha256::digest(body));

    let mut signed_headers = vec!["host", "x-amz-content-sha256", "x-amz-date"];
    let mut canonical_headers = format!("host:{host}\n");
    canonical_headers.push_str(&format!("x-amz-content-sha256:{payload_hash}\n"));
    canonical_headers.push_str(&format!("x-amz-date:{amz_date}\n"));
    if !content_type.is_empty() {
        signed_headers.push("content-type");
        canonical_headers.push_str(&format!("content-type:{content_type}\n"));
    }
    signed_headers.sort();
    let signed_header_list = signed_headers.join(";");

    let canonical_request =
        format!("{method}\n{path}\n\n{canonical_headers}\n{signed_header_list}\n{payload_hash}");

    let scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let mut mac = HmacSha256::new_from_slice(format!("AWS4{secret_key}").as_bytes())
        .map_err(|_| anyhow!("cannot create the signing key"))?;
    mac.update(date_stamp.as_bytes());
    let date_key = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&date_key).map_err(|_| anyhow!("bad key"))?;
    mac.update(region.as_bytes());
    let region_key = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&region_key).map_err(|_| anyhow!("bad key"))?;
    mac.update(b"s3");
    let service_key = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&service_key).map_err(|_| anyhow!("bad key"))?;
    mac.update(b"aws4_request");
    let signing_key = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&signing_key).map_err(|_| anyhow!("bad key"))?;
    mac.update(string_to_sign.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_header_list}, Signature={signature}"
    );

    let mut request = client.request(reqwest::Method::from_bytes(method.as_bytes())?, url);
    request = request
        .header("x-amz-date", amz_date)
        .header("x-amz-content-sha256", payload_hash)
        .header("authorization", authorization);
    if !content_type.is_empty() {
        request = request.header("content-type", content_type);
    }
    if !body.is_empty() {
        request = request.body(body.to_vec());
    }
    Ok(request.send().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_s3_endpoint_may_not_be_a_local_address() {
        assert!(check_s3_endpoint("https://s3.amazonaws.com").is_ok());
        assert!(check_s3_endpoint("https://minio.example.com:9000").is_ok());
        // The cloud metadata service and the loopback must never be reached
        // from a root process.
        assert!(check_s3_endpoint("https://169.254.169.254").is_err());
        assert!(check_s3_endpoint("https://127.0.0.1:9000").is_err());
        assert!(check_s3_endpoint("https://localhost:9000").is_err());
        assert!(check_s3_endpoint("https://10.0.0.5").is_err());
        assert!(check_s3_endpoint("https://192.168.1.10").is_err());
        // Plain HTTP would send the archive and the credentials in the clear.
        assert!(check_s3_endpoint("http://s3.amazonaws.com").is_err());
    }

    fn test_plan(dir: &Path) -> BackupPlan {
        BackupPlan {
            id: "test".into(),
            kind: Kind::Full,
            site_id: None,
            site_domain: None,
            site_root: None,
            contents: Contents {
                vhosts: true,
                pools: true,
                files: true,
                databases: false,
            },
            destination: Destination::Local {
                path: dir.join("backups"),
            },
            passphrase: None,
            staging_dir: dir.to_path_buf(),
            vhost_dir: dir.join("nginx"),
            pool_dir: dir.join("php"),
            www_dir: dir.join("www"),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sp-backup-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn creates_and_verifies_an_archive() {
        let dir = scratch("archive");
        std::fs::create_dir_all(dir.join("nginx")).unwrap();
        std::fs::write(dir.join("nginx/site.conf"), "server {}").unwrap();
        std::fs::create_dir_all(dir.join("www/demo")).unwrap();
        std::fs::write(dir.join("www/demo/index.html"), "hello").unwrap();

        let plan = test_plan(&dir);
        let result = create_backup(&plan).await.unwrap();
        assert!(result.size > 0);
        assert!(!result.encrypted);
        assert!(result.entries.iter().any(|e| e.starts_with("nginx/")));

        let archive = dir.join("backups").join(&result.name);
        assert!(archive.exists());
        let manifest = verify_archive(&archive).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.kind, "full");
    }

    #[tokio::test]
    async fn encrypts_and_decrypts_roundtrip() {
        let dir = scratch("encrypt");
        std::fs::create_dir_all(dir.join("nginx")).unwrap();
        std::fs::write(dir.join("nginx/site.conf"), "server {}").unwrap();

        let mut plan = test_plan(&dir);
        plan.passphrase = Some("correct horse battery staple".into());
        let result = create_backup(&plan).await.unwrap();
        assert!(result.encrypted);

        // The encrypted archive must not be a plain gzip stream.
        let raw = std::fs::read(dir.join("backups").join(&result.name)).unwrap();
        assert_eq!(&raw[..5], ENCRYPTED_MAGIC);

        // Decrypting with the wrong passphrase fails.
        let target = dir.join("decrypted.tar.gz");
        assert!(
            decrypt_file(
                &dir.join("backups").join(&result.name),
                &target,
                "wrong passphrase"
            )
            .is_err()
        );

        // With the right passphrase it round-trips.
        decrypt_file(
            &dir.join("backups").join(&result.name),
            &target,
            "correct horse battery staple",
        )
        .unwrap();
        let manifest = read_manifest(&target).unwrap();
        assert_eq!(manifest.version, 1);
    }

    #[tokio::test]
    async fn restores_the_tree() {
        let dir = scratch("restore");
        std::fs::create_dir_all(dir.join("nginx")).unwrap();
        std::fs::write(dir.join("nginx/site.conf"), "server {}").unwrap();

        let plan = test_plan(&dir);
        let result = create_backup(&plan).await.unwrap();
        let archive = dir.join("backups").join(&result.name);

        // Wipe the source, then restore.
        std::fs::remove_dir_all(dir.join("nginx")).unwrap();
        let restore = restore_archive(
            &archive,
            &dir.join("nginx"),
            &dir.join("php"),
            &dir.join("www"),
        )
        .unwrap();
        assert!(!restore.restored.is_empty());
        assert!(dir.join("nginx/site.conf").exists());
    }

    #[test]
    fn validates_destinations() {
        let local = Destination::Local {
            path: PathBuf::from("/var/backups"),
        };
        assert!(local.validate().is_ok());

        let sftp = Destination::Sftp {
            host: String::new(),
            port: 22,
            user: "root".into(),
            key_path: None,
            path: "/backups".into(),
        };
        assert!(sftp.validate().is_err());

        let s3 = Destination::S3 {
            endpoint: "https://s3.example".into(),
            region: "us-east-1".into(),
            bucket: "backups".into(),
            prefix: None,
            access_key: "key".into(),
            secret_key: "secret".into(),
        };
        assert!(s3.validate().is_ok());
    }

    #[test]
    fn archive_names_are_safe() {
        let name = archive_name("abc-123", "2026-01-02T03:04:05+00:00");
        assert!(name.starts_with("synthpanel-abc-123-"));
        assert!(name.ends_with(".tar.gz"));
        assert!(!name.contains(':'));
        assert!(!name.contains('+'));
    }

    #[test]
    fn checksum_is_stable() {
        let dir = scratch("checksum");
        let file = dir.join("data.bin");
        std::fs::write(&file, b"synthpanel").unwrap();
        let first = checksum_file(&file).unwrap();
        let second = checksum_file(&file).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn exclude_patterns_reject_path_traversal() {
        let allowed = [
            "cache",
            "cache/",
            "*.log",
            "storage/logs/*.log",
            "node_modules/**",
            "site-a/tmp",
        ];
        assert!(
            validate_exclude_patterns(
                &allowed
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            )
            .is_ok()
        );

        for bad in [
            "../secret",
            "foo/../../secret",
            "foo/./bar",
            "/etc/passwd",
            "",
            "   ",
            "foo\\bar",
            "foo\0bar",
        ] {
            assert!(
                validate_exclude_patterns(&[bad.to_string()]).is_err(),
                "`{bad}` should be rejected"
            );
        }
        assert!(
            Excludes::new(&["../secret".to_string()]).is_err(),
            "compilation repeats validation"
        );
    }

    #[test]
    fn exclude_globs_match_site_paths() {
        let excludes = Excludes::new(&[
            "cache".into(),
            "*.log".into(),
            "storage/**/tmp".into(),
            "docs/readme.md".into(),
        ])
        .unwrap();
        assert!(excludes.is_excluded(Path::new("cache")));
        assert!(excludes.is_excluded(Path::new("nested/cache/file.php")));
        assert!(excludes.is_excluded(Path::new("logs/app.log")));
        assert!(excludes.is_excluded(Path::new("storage/tmp")));
        assert!(excludes.is_excluded(Path::new("storage/2026/tmp")));
        assert!(excludes.is_excluded(Path::new("docs/readme.md")));
        assert!(!excludes.is_excluded(Path::new("docs/other.md")));
        assert!(!excludes.is_excluded(Path::new("public/index.html")));
        assert!(Excludes::default().is_empty());
    }

    #[tokio::test]
    async fn excludes_site_files_from_a_full_archive() {
        let dir = scratch("exclude-full");
        for site in ["one", "two"] {
            let root = dir.join("www").join(site);
            std::fs::create_dir_all(root.join("cache")).unwrap();
            std::fs::write(root.join("index.html"), "hello").unwrap();
            std::fs::write(root.join("cache/data.bin"), "cache").unwrap();
            std::fs::write(root.join("app.log"), "log").unwrap();
        }

        let plan = test_plan(&dir);
        let result = create_backup_with_excludes(&plan, &["cache".into(), "*.log".into()])
            .await
            .unwrap();
        let names = list_archive(&dir.join("backups").join(&result.name)).unwrap();
        assert!(names.iter().any(|name| name.ends_with("one/index.html")));
        assert!(names.iter().any(|name| name.ends_with("two/index.html")));
        assert!(!names.iter().any(|name| name.contains("cache")));
        assert!(!names.iter().any(|name| name.ends_with(".log")));
        assert!(
            result
                .entries
                .iter()
                .any(|entry| entry.ends_with("one/index.html"))
        );
        assert!(!result.entries.iter().any(|entry| entry.contains("cache")));
    }

    #[tokio::test]
    async fn excludes_apply_to_a_site_backup() {
        let dir = scratch("exclude-site");
        let root = dir.join("www/one");
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::write(root.join("index.html"), "hello").unwrap();
        std::fs::write(root.join("cache/data.bin"), "cache").unwrap();

        let mut plan = test_plan(&dir);
        plan.kind = Kind::Site;
        plan.site_id = Some("one".into());
        plan.site_root = Some(root.clone());
        let result = create_backup_with_excludes(&plan, &["cache".into()])
            .await
            .unwrap();
        let names = list_archive(&dir.join("backups").join(&result.name)).unwrap();
        assert!(names.iter().any(|name| name.ends_with("one/index.html")));
        assert!(!names.iter().any(|name| name.contains("cache")));
    }

    #[tokio::test]
    async fn backups_without_excludes_keep_every_file() {
        let dir = scratch("exclude-compat");
        let root = dir.join("www/one");
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::write(root.join("index.html"), "hello").unwrap();
        std::fs::write(root.join("cache/data.bin"), "cache").unwrap();

        let plan = test_plan(&dir);
        let result = create_backup(&plan).await.unwrap();
        let names = list_archive(&dir.join("backups").join(&result.name)).unwrap();
        assert!(names.iter().any(|name| name.contains("cache/data.bin")));
        assert!(names.iter().any(|name| name.ends_with("one/index.html")));
    }

    #[tokio::test]
    async fn excludes_are_stored_per_scope() {
        let dir = scratch("exclude-store");
        let pool = crate::db::connect(dir.join("state.db")).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();

        set_excludes(&pool, None, &["cache".into()]).await.unwrap();
        set_excludes(&pool, Some("one".into()), &["*.log".into()])
            .await
            .unwrap();
        assert_eq!(
            effective_excludes(&pool, None).await.unwrap(),
            vec!["cache"]
        );
        assert_eq!(
            effective_excludes(&pool, Some("one")).await.unwrap(),
            vec!["cache", "*.log"]
        );
        assert_eq!(
            effective_excludes(&pool, Some("two")).await.unwrap(),
            vec!["cache"]
        );

        // An empty list clears the scope without touching the other one.
        set_excludes(&pool, Some("one".into()), &[]).await.unwrap();
        assert_eq!(
            effective_excludes(&pool, Some("one")).await.unwrap(),
            vec!["cache"]
        );
        assert!(
            set_excludes(&pool, None, &["../secret".into()])
                .await
                .is_err()
        );
    }

    #[test]
    fn validates_and_labels_rclone_destinations() {
        let remote = Destination::Rclone {
            remote: "gdrive".into(),
            path: "backups/synthpanel".into(),
        };
        assert!(remote.validate().is_ok());
        assert_eq!(remote.label(), "rclone://gdrive/backups/synthpanel");

        for (name, path) in [
            ("", "backups"),
            ("bad:name", "backups"),
            ("bad/name", "backups"),
            ("-flag", "backups"),
            ("gdrive", ""),
            ("gdrive", "-flag"),
        ] {
            assert!(
                Destination::Rclone {
                    remote: name.into(),
                    path: path.into(),
                }
                .validate()
                .is_err(),
                "`{name}:{path}` should be rejected"
            );
        }
    }

    #[test]
    fn rclone_locations_roundtrip() {
        let location = rclone_location("gdrive", "/backups/sp/", "archive.tar.gz");
        assert_eq!(location, "rclone://gdrive/backups/sp/archive.tar.gz");
        assert_eq!(
            rclone_remote_from_location(&location).unwrap(),
            "gdrive:backups/sp/archive.tar.gz"
        );
        assert_eq!(
            rclone_remote_from_location("gdrive:backups/archive.tar.gz").unwrap(),
            "gdrive:backups/archive.tar.gz"
        );
        assert!(rclone_remote_from_location("gdrive").is_err());
        assert_eq!(rclone_target("gdrive", "/backups"), "gdrive:backups");
    }

    #[cfg(unix)]
    #[test]
    fn rclone_runs_the_configured_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("rclone-bin");
        let capture = dir.join("args.txt");
        let bin = dir.join("fake-rclone");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", capture.display()),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&bin).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bin, permissions).unwrap();

        let local = dir.join("archive.tar.gz");
        run_rclone_with(
            &bin,
            &[
                "copyto",
                &local.to_string_lossy(),
                "gdrive:backups/archive.tar.gz",
            ],
        )
        .unwrap();
        let args = std::fs::read_to_string(&capture).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            vec![
                "copyto",
                local.to_str().unwrap(),
                "gdrive:backups/archive.tar.gz"
            ]
        );
    }
}
