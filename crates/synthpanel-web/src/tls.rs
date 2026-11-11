//! Panel TLS material: load an existing certificate or generate a self-signed
//! one on first start.

use anyhow::{Context, Result, bail};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing::warn;

/// The state directory the panel falls back to when the TLS directory is not
/// writable by the web user.
const DEFAULT_STATE_DIR: &str = "/var/lib/synthpanel";

/// A certificate resolver that picks the certificate up from disk.
///
/// The panel can issue a new certificate for itself while it is running, so
/// the resolver checks the files on every handshake and reloads them when
/// they changed.
#[derive(Debug)]
pub struct PanelCertificates {
    cert_path: PathBuf,
    key_path: PathBuf,
    cached: Mutex<Option<(Arc<CertifiedKey>, SystemTime)>>,
}

impl PanelCertificates {
    pub fn new(cert_path: PathBuf, key_path: PathBuf) -> Self {
        Self {
            cert_path,
            key_path,
            cached: Mutex::new(None),
        }
    }

    /// The rustls configuration for the panel.
    pub fn server_config(&self) -> Result<rustls::ServerConfig> {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        Ok(rustls::ServerConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(Self::new(
                self.cert_path.clone(),
                self.key_path.clone(),
            ))))
    }

    /// Load the certificate and key, reusing the cached copy when the files
    /// did not change.
    fn load(&self) -> Option<Arc<CertifiedKey>> {
        let modified = std::fs::metadata(&self.cert_path)
            .and_then(|meta| meta.modified())
            .ok()?;

        let mut cached = self.cached.lock().ok()?;
        if let Some((key, stamp)) = cached.as_ref() {
            if *stamp == modified {
                return Some(key.clone());
            }
        }

        let key = match load_certified_key(&self.cert_path, &self.key_path) {
            Ok(key) => key,
            Err(error) => {
                warn!(
                    %error,
                    cert = %self.cert_path.display(),
                    "cannot load the panel certificate"
                );
                return None;
            }
        };
        *cached = Some((key.clone(), modified));
        Some(key)
    }
}

impl ResolvesServerCert for PanelCertificates {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.load()
    }
}

/// Read a certificate chain and its key from PEM files.
fn load_certified_key(cert_path: &Path, key_path: &Path) -> Result<Arc<CertifiedKey>> {
    let cert_file = std::fs::File::open(cert_path)
        .with_context(|| format!("cannot open {}", cert_path.display()))?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let chain = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("cannot parse {}", cert_path.display()))?;
    if chain.is_empty() {
        bail!("the certificate file {} is empty", cert_path.display());
    }

    let key_file = std::fs::File::open(key_path)
        .with_context(|| format!("cannot open {}", key_path.display()))?;
    let mut key_reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .with_context(|| format!("cannot parse {}", key_path.display()))?
        .ok_or_else(|| anyhow::anyhow!("the key file {} has no key", key_path.display()))?;
    let signing = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)
        .context("the private key is not supported")?;

    Ok(Arc::new(CertifiedKey {
        cert: chain,
        key: signing,
        ocsp: None,
    }))
}

pub fn ensure_certificate(cert_path: &Path, key_path: &Path, hostname: &str) -> Result<bool> {
    if cert_path.exists() && key_path.exists() && certificate_matches(cert_path, hostname) {
        return Ok(false);
    }
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "cannot create the TLS directory {} — it must be writable by the web user",
                parent.display()
            )
        })?;
    }

    let mut names = vec!["localhost".to_string()];
    if !hostname.is_empty() && hostname != "localhost" {
        names.push(hostname.to_string());
    }

    let certified = rcgen::generate_simple_self_signed(names)
        .context("failed to generate a self-signed certificate")?;
    std::fs::write(cert_path, certified.cert.pem()).with_context(|| {
        format!(
            "cannot write the panel certificate to {} — the directory must be writable by the \
             web user",
            cert_path.display()
        )
    })?;
    std::fs::write(key_path, certified.signing_key.serialize_pem()).with_context(|| {
        format!(
            "cannot write the panel key to {} — the directory must be writable by the web user",
            key_path.display()
        )
    })?;
    restrict_permissions(key_path)?;
    restrict_permissions(cert_path)?;
    remember_hostname(cert_path, hostname)?;
    Ok(true)
}

/// Whether the certificate on disk was issued for the configured hostname.
///
/// The hostname is stored next to the certificate, so a changed panel
/// hostname produces a new certificate instead of a name mismatch in the
/// browser. A certificate without a stamp comes from an older version and is
/// replaced once.
fn certificate_matches(cert_path: &Path, hostname: &str) -> bool {
    let Some(stamp) = hostname_stamp_path(cert_path) else {
        return false;
    };
    match std::fs::read_to_string(&stamp) {
        Ok(stored) => stored.trim() == hostname.trim(),
        Err(_) => false,
    }
}

/// Remember the hostname a certificate was issued for.
fn remember_hostname(cert_path: &Path, hostname: &str) -> Result<()> {
    let Some(stamp) = hostname_stamp_path(cert_path) else {
        return Ok(());
    };
    std::fs::write(&stamp, hostname).with_context(|| {
        format!(
            "cannot write the certificate hostname to {}",
            stamp.display()
        )
    })
}

/// The file that records the hostname of a certificate.
fn hostname_stamp_path(cert_path: &Path) -> Option<std::path::PathBuf> {
    cert_path
        .parent()
        .map(|dir| dir.join("panel-cert.hostname"))
}

/// Pick a directory the web process can write the panel certificate to.
///
/// The certificate belongs to the unprivileged web process, so its directory
/// has to be writable by that user. When the configured directory is not
/// writable — a mis-installed package, for example — the panel falls back to
/// its own state directory instead of refusing to start.
pub fn prepare_tls_dir(configured: &Path) -> PathBuf {
    if writable(configured) {
        return configured.to_path_buf();
    }

    let state = std::env::var("SYNTHPANEL_STATE_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STATE_DIR.to_string());
    let fallback = Path::new(&state).join("tls");
    if writable(&fallback) {
        warn!(
            configured = %configured.display(),
            fallback = %fallback.display(),
            "the TLS directory is not writable by the web user; using the panel state directory"
        );
        return fallback;
    }

    // Neither directory works: return the configured one so the caller fails
    // with a message that names it.
    configured.to_path_buf()
}

/// Whether a directory exists (or can be created) and accepts new files.
fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".synthpanel-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.extension().and_then(|e| e.to_str()) == Some("key") {
        0o600
    } else {
        0o644
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writable_directory_is_kept() {
        let dir = std::env::temp_dir().join(format!("sp-tls-{}", uuid::Uuid::new_v4()));
        let chosen = prepare_tls_dir(&dir);
        assert_eq!(chosen, dir);
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_fallback_is_used_when_the_directory_is_read_only() {
        // A file where a directory is expected can never be written to.
        let file = std::env::temp_dir().join(format!("sp-tls-file-{}", uuid::Uuid::new_v4()));
        std::fs::write(&file, b"x").unwrap();
        let state = std::env::temp_dir().join(format!("sp-tls-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&state).unwrap();
        unsafe { std::env::set_var("SYNTHPANEL_STATE_DIR", &state) };
        let chosen = prepare_tls_dir(&file);
        assert_eq!(chosen, state.join("tls"));
        unsafe { std::env::remove_var("SYNTHPANEL_STATE_DIR") };
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn a_changed_hostname_produces_a_new_certificate() {
        let dir = std::env::temp_dir().join(format!("sp-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("panel-cert.pem");
        let key = dir.join("panel-key.pem");

        assert!(ensure_certificate(&cert, &key, "panel.example.com").unwrap());
        // The same hostname keeps the certificate.
        assert!(!ensure_certificate(&cert, &key, "panel.example.com").unwrap());
        // A different hostname replaces it.
        assert!(ensure_certificate(&cert, &key, "other.example.com").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
