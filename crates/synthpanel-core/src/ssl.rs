//! Let's Encrypt certificate issuance and renewal.
//!
//! The panel drives `acme.sh` through its well-known CLI rather than
//! embedding a full ACME client: acme.sh is already a dependency of most
//! server stacks, supports HTTP-01 and DNS-01, and can renew without a
//! long-running process.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

/// Where acme.sh stores its certificates.
#[derive(Debug, Clone)]
pub struct AcmePaths {
    /// Home directory of the acme.sh installation.
    pub home: PathBuf,
    /// Directory that holds the account configuration.
    pub conf_dir: PathBuf,
    /// Directory where issued certificates live.
    pub cert_dir: PathBuf,
    /// The acme.sh executable.
    pub binary: PathBuf,
}

impl AcmePaths {
    pub fn from_env() -> Self {
        let home = std::env::var_os("SYNTHPANEL_ACME_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/root/.acme.sh"));
        Self {
            conf_dir: home.join("conf"),
            cert_dir: home.join("certs"),
            binary: home.join("acme.sh"),
            home,
        }
    }

    /// Certificate paths for a domain, following acme.sh's layout.
    pub fn domain_paths(&self, domain: &str) -> DomainCertPaths {
        DomainCertPaths {
            fullchain: self.cert_dir.join(domain).join("fullchain.cer"),
            privkey: self.cert_dir.join(domain).join("example.privkey"),
            conf: self.conf_dir.join(format!("{domain}.conf")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DomainCertPaths {
    pub fullchain: PathBuf,
    pub privkey: PathBuf,
    pub conf: PathBuf,
}

impl DomainCertPaths {
    /// The paths nginx expects, i.e. the stable `/etc/letsencrypt/live`
    /// layout that the vhost template references.
    pub fn live_paths(&self) -> (PathBuf, PathBuf) {
        let domain = self
            .conf
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let live = Path::new("/etc/letsencrypt/live").join(domain);
        (live.join("fullchain.pem"), live.join("privkey.pem"))
    }
}

/// The outcome of an issuance attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuanceResult {
    pub domain: String,
    pub issued: bool,
    pub expires_at: Option<String>,
    pub message: String,
}

/// Issue a certificate for `domain` over HTTP-01.
///
/// The webroot must already serve `/.well-known/acme-challenge/` for the
/// domain, which is what the panel's HTTP vhost does.
pub async fn issue_http01(
    paths: &AcmePaths,
    domain: &str,
    webroot: &Path,
    email: Option<&str>,
) -> Result<IssuanceResult> {
    if !paths.binary.exists() {
        bail!(
            "acme.sh is not installed at {}; install it or use a custom certificate",
            paths.binary.display()
        );
    }

    let domain_paths = paths.domain_paths(domain);
    let mut command = Command::new(&paths.binary);
    command.arg("--issue");
    command.arg("--server").arg("letsencrypt");
    command.arg("--domain").arg(domain);
    command.arg("--webroot").arg(webroot);
    command.arg("--keylength").arg("ec-256");
    command.arg("--force"); // re-issue even when a valid cert exists
    if let Some(email) = email {
        command.arg("--accountemail").arg(email);
    }

    let output = command.output().context("failed to run acme.sh")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        warn!(domain, %stderr, "acme.sh failed");
        return Ok(IssuanceResult {
            domain: domain.to_string(),
            issued: false,
            expires_at: None,
            message: if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            },
        });
    }

    // acme.sh stores the certificate under its own layout; publish it in the
    // stable /etc/letsencrypt/live/<domain>/ location that the vhost uses.
    publish_certificate(&domain_paths)?;

    info!(domain, "certificate issued");
    Ok(IssuanceResult {
        domain: domain.to_string(),
        issued: true,
        expires_at: read_expiry(&domain_paths.fullchain),
        message: "certificate issued".to_string(),
    })
}

/// Copy the issued certificate into the stable `/etc/letsencrypt/live` layout.
pub fn publish_certificate(domain_paths: &DomainCertPaths) -> Result<()> {
    let (fullchain, privkey) = domain_paths.live_paths();
    if let Some(parent) = fullchain.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::copy(&domain_paths.fullchain, &fullchain)
        .with_context(|| format!("cannot copy {}", domain_paths.fullchain.display()))?;
    std::fs::copy(&domain_paths.privkey, &privkey)
        .with_context(|| format!("cannot copy {}", domain_paths.privkey.display()))?;
    Ok(())
}

/// Read the `notAfter` date from a PEM certificate.
pub fn read_expiry(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let pem = extract_pem(&contents, "CERTIFICATE")?;
    let der = base64_decode(&pem).ok()?;
    // Minimal DER walk: find the validity `notAfter` (UTCTime/GeneralizedTime).
    let text = der_time_string(&der)?;
    Some(text)
}

fn extract_pem(contents: &str, label: &str) -> Option<String> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = contents.find(&begin)? + begin.len();
    let stop = contents[start..].find(&end)? + start;
    Some(contents[start..stop].trim().to_string())
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|error| anyhow!("cannot decode base64: {error}"))
}

/// Find the first printable UTC timestamp in a DER blob.
///
/// Certificates encode `notAfter` as a UTCTime (`YYMMDDHHMMSSZ`) or a
/// GeneralizedTime (`YYYYMMDDHHMMSSZ`). We scan for either pattern and
/// convert it to an RFC 3339 string.
fn der_time_string(der: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(der);
    let bytes: Vec<char> = text.chars().collect();

    // GeneralizedTime: YYYYMMDDHHMMSSZ (15 chars)
    for window in bytes.windows(15) {
        let candidate: String = window.iter().collect();
        if candidate.ends_with('Z')
            && candidate[..14].chars().all(|c| c.is_ascii_digit())
            && is_plausible_timestamp(&candidate[..14])
        {
            return Some(to_rfc3339(&candidate[..14]));
        }
    }
    // UTCTime: YYMMDDHHMMSSZ (13 chars)
    for window in bytes.windows(13) {
        let candidate: String = window.iter().collect();
        if candidate.ends_with('Z')
            && candidate[..12].chars().all(|c| c.is_ascii_digit())
            && is_plausible_timestamp(&candidate[..12])
        {
            return Some(to_rfc3339(&candidate[..12]));
        }
    }
    None
}

fn is_plausible_timestamp(digits: &str) -> bool {
    if digits.len() == 14 {
        let month: u32 = digits[4..6].parse().unwrap_or(0);
        let day: u32 = digits[6..8].parse().unwrap_or(0);
        (1..=12).contains(&month) && (1..=31).contains(&day)
    } else if digits.len() == 12 {
        let month: u32 = digits[2..4].parse().unwrap_or(0);
        let day: u32 = digits[4..6].parse().unwrap_or(0);
        (1..=12).contains(&month) && (1..=31).contains(&day)
    } else {
        false
    }
}

fn to_rfc3339(digits: &str) -> String {
    // GeneralizedTime: YYYYMMDDHHMMSS
    // UTCTime:        YYMMDDHHMMSS
    let (year, rest) = if digits.len() == 14 {
        let year: u32 = digits[0..4].parse().unwrap_or(1970);
        (year, &digits[4..])
    } else {
        let short: u32 = digits[0..2].parse().unwrap_or(0);
        let year = if short >= 70 {
            1900 + short
        } else {
            2000 + short
        };
        (year, &digits[2..])
    };
    format!(
        "{year:04}-{}-{}T{}:{}:{}Z",
        &rest[0..2],  // month
        &rest[2..4],  // day
        &rest[4..6],  // hour
        &rest[6..8],  // minute
        &rest[8..10], // second
    )
}

/// Whether a certificate expires within `days`.
pub fn expires_within(path: &Path, days: i64) -> bool {
    match read_expiry(path) {
        Some(expiry) => match chrono::DateTime::parse_from_rfc3339(&expiry) {
            Ok(when) => {
                let threshold = chrono::Utc::now() + chrono::Duration::days(days);
                when < threshold
            }
            Err(_) => false,
        },
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_pem_block() {
        let pem = "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n";
        assert_eq!(extract_pem(pem, "CERTIFICATE").as_deref(), Some("QUJD"));
        assert!(extract_pem(pem, "PRIVATE KEY").is_none());
    }

    #[test]
    fn finds_generalized_time() {
        // "20260101120000Z" embedded in a DER-ish blob.
        let mut der = b"\x30\x03".to_vec();
        der.extend_from_slice(b"20260101120000Z");
        assert_eq!(
            der_time_string(&der).as_deref(),
            Some("2026-01-01T12:00:00Z")
        );
    }

    #[test]
    fn finds_utc_time() {
        let mut der = b"\x17\x0d".to_vec();
        der.extend_from_slice(b"260101120000Z");
        assert_eq!(
            der_time_string(&der).as_deref(),
            Some("2026-01-01T12:00:00Z")
        );
    }

    #[test]
    fn rejects_implausible_timestamps() {
        let mut der = b"\x30\x03".to_vec();
        der.extend_from_slice(b"20269901120000Z");
        assert!(der_time_string(&der).is_none());
    }

    #[test]
    fn missing_certificate_is_not_expiring() {
        assert!(!expires_within(Path::new("/nonexistent/cert.pem"), 30));
    }
}
