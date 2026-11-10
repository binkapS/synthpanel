//! Panel TLS material: load an existing certificate or generate a self-signed
//! one on first start.

use anyhow::{Context, Result};
use std::path::Path;

pub fn ensure_certificate(cert_path: &Path, key_path: &Path, hostname: &str) -> Result<bool> {
    if cert_path.exists() && key_path.exists() {
        return Ok(false);
    }
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut names = vec!["localhost".to_string()];
    if !hostname.is_empty() && hostname != "localhost" {
        names.push(hostname.to_string());
    }

    let certified = rcgen::generate_simple_self_signed(names)
        .context("failed to generate a self-signed certificate")?;
    std::fs::write(cert_path, certified.cert.pem())?;
    std::fs::write(key_path, certified.signing_key.serialize_pem())?;
    restrict_permissions(key_path)?;
    restrict_permissions(cert_path)?;
    Ok(true)
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
