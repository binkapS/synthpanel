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
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
            }
        }
        Ok(())
    }
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

/// Create a backup and upload it to the destination.
pub async fn create_backup(plan: &BackupPlan) -> Result<BackupResult> {
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
                copy_tree(root, &work_dir.join("www").join(name))?;
            }
        } else {
            match &plan.site_id {
                Some(site_id) => {
                    let source = plan.www_dir.join(site_id);
                    if source.exists() {
                        copy_tree(&source, &work_dir.join("www").join(site_id))?;
                    }
                }
                None => {
                    copy_tree(&plan.www_dir, &work_dir.join("www"))?;
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

/// Copy a directory tree into the staging area.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let target = destination.join(&name);
        let _ = name;
        if path.is_dir() {
            copy_tree(&path, &target)?;
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
            upload_sftp(host, *port, user, key_path.as_deref(), local, &target)?;
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
            download_sftp(host, *port, user, key_path.as_deref(), remote, local)?;
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
}
