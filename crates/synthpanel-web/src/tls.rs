//! Panel TLS material: load an existing certificate or generate a self-signed
//! one on first start.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

/// The state directory the panel falls back to when the TLS directory is not
/// writable by the web user.
const DEFAULT_STATE_DIR: &str = "/var/lib/synthpanel";

pub fn ensure_certificate(cert_path: &Path, key_path: &Path, hostname: &str) -> Result<bool> {
    if cert_path.exists() && key_path.exists() {
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
    Ok(true)
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
}
