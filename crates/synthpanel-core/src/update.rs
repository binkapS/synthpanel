//! Self-update with signature verification and rollback.
//!
//! The update source publishes an *envelope*:
//!
//! ```json
//! { "payload": "{\"version\":\"0.2.0\",...}", "signature": "<base64 ed25519>" }
//! ```
//!
//! The signature covers the exact bytes of `payload`, so the manifest needs no
//! canonical JSON encoding. The archive is downloaded, its SHA-256 checked
//! against the signed manifest, then the binaries are replaced. The previous
//! binaries are kept so a failed health check can roll back.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The default update channel document.
pub const DEFAULT_UPDATE_URL: &str = "https://updates.synthpanel.io/stable.json";

/// The signed description of a release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default)]
    pub channel: String,
    /// Where the `.tar.gz` containing the binaries lives.
    pub url: String,
    /// Lower-case hex SHA-256 of the archive.
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub published_at: String,
}

/// The envelope served by the update endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEnvelope {
    pub payload: String,
    pub signature: String,
}

/// The result of an update check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub channel: String,
    pub source: String,
    pub update_available: bool,
    pub release: Option<ReleaseManifest>,
}

/// Everything needed to apply an update.
#[derive(Debug, Clone)]
pub struct UpdatePlan {
    pub manifest: ReleaseManifest,
    /// Directory holding the installed binaries.
    pub install_dir: PathBuf,
    /// Where the new release is unpacked.
    pub staging_dir: PathBuf,
    /// Where the previous binaries are kept for rollback.
    pub backup_dir: PathBuf,
    /// When false, systemd units are not restarted (tests, containers).
    pub restart_services: bool,
    /// Optional health-check command, run after the binaries are replaced.
    pub health_check: Option<String>,
}

/// The outcome of an applied update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateResult {
    pub from_version: String,
    pub to_version: String,
    pub backup_dir: String,
    pub health_check: String,
}

/// The binaries managed by an update.
pub const BINARIES: [&str; 3] = ["synthpanel-core", "synthpanel-web", "synthpanel"];

/// The configured update source (env override, otherwise the default).
pub fn update_source() -> String {
    std::env::var("SYNTHPANEL_UPDATE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATE_URL.to_string())
}

/// The Ed25519 public key trusted for update signatures (hex).
pub fn update_public_key() -> String {
    std::env::var("SYNTHPANEL_UPDATE_PUBKEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default()
}

/// Fetch the update document from a URL or a local path.
pub async fn fetch_envelope(source: &str) -> Result<UpdateEnvelope> {
    let text = if let Some(path) = local_path(source) {
        std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read the update document at {}", path.display()))?
    } else {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let response = client
            .get(source)
            .send()
            .await
            .with_context(|| format!("cannot fetch {source}"))?;
        if !response.status().is_success() {
            bail!("the update endpoint returned {}", response.status());
        }
        response.text().await?
    };
    serde_json::from_str(&text).context("the update document is not valid JSON")
}

/// Interpret a source that is really a local file.
pub fn local_path(source: &str) -> Option<PathBuf> {
    if let Some(rest) = source.strip_prefix("file://") {
        return Some(PathBuf::from(rest));
    }
    if source.contains("://") {
        return None;
    }
    let path = PathBuf::from(source);
    path.is_file().then_some(path)
}

/// Verify the envelope signature and return the manifest.
pub fn verify_envelope(envelope: &UpdateEnvelope, public_key_hex: &str) -> Result<ReleaseManifest> {
    let key =
        hex::decode(public_key_hex.trim()).context("the update public key is not valid hex")?;
    if key.len() != 32 {
        bail!("the update public key must be 32 bytes");
    }
    let signature = BASE64
        .decode(envelope.signature.trim())
        .context("the update signature is not valid base64")?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(envelope.payload.as_bytes(), &signature)
        .map_err(|_| anyhow::anyhow!("the update signature is invalid"))?;
    serde_json::from_str(&envelope.payload).context("the signed payload is not a manifest")
}

/// Check for a newer release.
pub async fn check(current_version: &str, channel: &str, source: &str) -> Result<UpdateCheck> {
    let envelope = fetch_envelope(source).await?;
    let public_key = update_public_key();
    if public_key.is_empty() {
        bail!(
            "no update public key is configured; set SYNTHPANEL_UPDATE_PUBKEY to the release key"
        );
    }
    let manifest = verify_envelope(&envelope, &public_key)?;
    if !manifest.channel.is_empty() && manifest.channel != channel {
        return Ok(UpdateCheck {
            current_version: current_version.to_string(),
            channel: channel.to_string(),
            source: source.to_string(),
            update_available: false,
            release: None,
        });
    }
    let update_available = is_newer(&manifest.version, current_version);
    Ok(UpdateCheck {
        current_version: current_version.to_string(),
        channel: channel.to_string(),
        source: source.to_string(),
        update_available,
        release: update_available.then_some(manifest),
    })
}

/// Compare dotted numeric versions (`1.2.10` > `1.2.9`).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn parse_version(value: &str) -> Vec<u64> {
    value
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// Download an archive and verify its checksum.
pub async fn download_verified(manifest: &ReleaseManifest, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(path) = local_path(&manifest.url) {
        std::fs::copy(&path, target)
            .with_context(|| format!("cannot copy the update archive {}", path.display()))?;
    } else {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let response = client.get(&manifest.url).send().await?;
        if !response.status().is_success() {
            bail!("the update archive returned {}", response.status());
        }
        let bytes = response.bytes().await?;
        std::fs::write(target, &bytes)?;
    }
    let digest = sha256_file(target)?;
    if !digest.eq_ignore_ascii_case(manifest.sha256.trim()) {
        let _ = std::fs::remove_file(target);
        bail!(
            "the update archive checksum does not match (expected {}, got {digest})",
            manifest.sha256
        );
    }
    Ok(())
}

/// SHA-256 of a file as lower-case hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
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

/// Unpack an update archive into `destination`, rejecting path traversal.
pub fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
            bail!(
                "the update archive contains an unsafe path: {}",
                path.display()
            );
        }
        if !entry.unpack_in(destination)? {
            bail!(
                "the update archive escapes its destination: {}",
                path.display()
            );
        }
    }
    Ok(())
}

/// Apply an update: verify, back up, replace, restart, health-check, roll back.
pub async fn apply(plan: &UpdatePlan, current_version: &str) -> Result<UpdateResult> {
    let manifest = &plan.manifest;
    let work = plan
        .staging_dir
        .join(format!("update-{}", manifest.version));
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    std::fs::create_dir_all(&work)?;

    let archive = work.join("release.tar.gz");
    download_verified(manifest, &archive).await?;

    let unpacked = work.join("unpacked");
    extract_archive(&archive, &unpacked)?;

    // Locate the binaries in the archive, allowing a single top-level directory.
    let mut sources = Vec::new();
    for name in BINARIES {
        let direct = unpacked.join(name);
        if direct.is_file() {
            sources.push((name, direct));
            continue;
        }
        let nested = std::fs::read_dir(&unpacked)?
            .flatten()
            .map(|entry| entry.path().join(name))
            .find(|path| path.is_file());
        match nested {
            Some(path) => sources.push((name, path)),
            None => bail!("the update archive does not contain `{name}`"),
        }
    }

    let backup = plan.backup_dir.join(format!("rollback-{current_version}"));
    std::fs::create_dir_all(&backup)?;
    for name in BINARIES {
        let installed = plan.install_dir.join(name);
        if installed.exists() {
            std::fs::copy(&installed, backup.join(name))?;
        }
    }

    install(&sources, &plan.install_dir)?;

    let health = match &plan.health_check {
        Some(command) => run_health_check(command, &plan.install_dir),
        None => Ok(()),
    };
    if let Err(error) = health {
        rollback(&backup, &plan.install_dir)?;
        if plan.restart_services {
            let _ = restart_services();
        }
        bail!("the update health check failed and was rolled back: {error}");
    }

    if plan.restart_services {
        if let Err(error) = restart_services() {
            rollback(&backup, &plan.install_dir)?;
            let _ = restart_services();
            bail!("the services failed to restart and the update was rolled back: {error}");
        }
    }

    Ok(UpdateResult {
        from_version: current_version.to_string(),
        to_version: manifest.version.clone(),
        backup_dir: backup.to_string_lossy().to_string(),
        health_check: plan.health_check.clone().unwrap_or_default(),
    })
}

fn install(sources: &[(&str, PathBuf)], install_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(install_dir)?;
    for (name, source) in sources {
        let target = install_dir.join(name);
        let temporary = install_dir.join(format!(".{name}.new"));
        std::fs::copy(source, &temporary)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&temporary, &target)?;
    }
    Ok(())
}

/// Copy the saved binaries back over the installed ones.
pub fn rollback(backup: &Path, install_dir: &Path) -> Result<()> {
    for name in BINARIES {
        let source = backup.join(name);
        if source.is_file() {
            let target = install_dir.join(name);
            let temporary = install_dir.join(format!(".{name}.rollback"));
            std::fs::copy(&source, &temporary)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&temporary, &target)?;
        }
    }
    Ok(())
}

fn run_health_check(command: &str, install_dir: &Path) -> Result<()> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(install_dir)
        .status()
        .with_context(|| format!("cannot run the health check `{command}`"))?;
    if !status.success() {
        bail!("`{command}` exited with {status}");
    }
    Ok(())
}

/// Restart the core and web units after a binary change.
pub fn restart_services() -> Result<()> {
    if std::env::var("SYNTHPANEL_SKIP_SYSTEMD").is_ok() {
        return Ok(());
    }
    let status = std::process::Command::new("systemctl")
        .args(["restart", "synthpanel-core", "synthpanel-web"])
        .status()
        .context("cannot run systemctl")?;
    if !status.success() {
        bail!("systemctl restart failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::KeyPair;

    fn envelope(version: &str) -> (UpdateEnvelope, String) {
        let rng = SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let manifest = ReleaseManifest {
            version: version.to_string(),
            channel: "stable".into(),
            url: "https://example.invalid/release.tar.gz".into(),
            sha256: "00".repeat(32),
            notes: String::new(),
            published_at: String::new(),
        };
        let payload = serde_json::to_string(&manifest).unwrap();
        let signature = BASE64.encode(pair.sign(payload.as_bytes()).as_ref());
        (
            UpdateEnvelope { payload, signature },
            hex::encode(pair.public_key().as_ref()),
        )
    }

    #[test]
    fn verifies_a_valid_envelope() {
        let (envelope, public_key) = envelope("0.2.0");
        let manifest = verify_envelope(&envelope, &public_key).unwrap();
        assert_eq!(manifest.version, "0.2.0");
    }

    #[test]
    fn rejects_tampering() {
        let (mut envelope, public_key) = envelope("0.2.0");
        envelope.payload = envelope.payload.replace("0.2.0", "9.9.9");
        assert!(verify_envelope(&envelope, &public_key).is_err());
    }

    #[test]
    fn rejects_a_missing_key() {
        let (envelope, _) = envelope("0.2.0");
        assert!(verify_envelope(&envelope, "").is_err());
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("1.2.10", "1.2.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("v2.0.0", "1.99.99"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sp-update-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn write_archive(dir: &Path, body: &str) -> PathBuf {
        let source = dir.join("src");
        std::fs::create_dir_all(&source).unwrap();
        for name in BINARIES {
            std::fs::write(source.join(name), body).unwrap();
        }
        let archive = dir.join("release.tar.gz");
        let file = std::fs::File::create(&archive).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for name in BINARIES {
            builder
                .append_path_with_name(source.join(name), name)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        archive
    }

    fn plan(dir: &Path, archive: &Path, health: &str) -> UpdatePlan {
        UpdatePlan {
            manifest: ReleaseManifest {
                version: "0.2.0".into(),
                channel: "stable".into(),
                url: archive.to_string_lossy().to_string(),
                sha256: sha256_file(archive).unwrap(),
                notes: String::new(),
                published_at: String::new(),
            },
            install_dir: dir.join("bin"),
            staging_dir: dir.join("staging"),
            backup_dir: dir.join("backups"),
            restart_services: false,
            health_check: Some(health.to_string()),
        }
    }

    #[tokio::test]
    async fn applies_and_keeps_a_rollback_copy() {
        let dir = temp_dir("apply");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for name in BINARIES {
            std::fs::write(dir.join("bin").join(name), "old").unwrap();
        }
        let archive = write_archive(&dir, "new");
        let plan = plan(&dir, &archive, "true");
        let result = apply(&plan, "0.1.0").await.unwrap();
        assert_eq!(result.to_version, "0.2.0");
        assert_eq!(
            std::fs::read_to_string(dir.join("bin/synthpanel-core")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("backups/rollback-0.1.0/synthpanel-core")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rolls_back_when_the_health_check_fails() {
        let dir = temp_dir("rollback");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for name in BINARIES {
            std::fs::write(dir.join("bin").join(name), "old").unwrap();
        }
        let archive = write_archive(&dir, "new");
        let plan = plan(&dir, &archive, "false");
        assert!(apply(&plan, "0.1.0").await.is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("bin/synthpanel-core")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_a_checksum_mismatch() {
        let dir = temp_dir("checksum");
        let archive = write_archive(&dir, "new");
        let mut plan = plan(&dir, &archive, "true");
        plan.manifest.sha256 = "00".repeat(32);
        assert!(apply(&plan, "0.1.0").await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
