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

/// The certificate the panel serves on its own interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelCertificate {
    /// The hostname the panel is reachable under.
    pub hostname: String,
    /// Where the certificate lives.
    pub path: String,
    /// Whether a certificate is installed at all.
    pub exists: bool,
    /// Whether the certificate was issued by the panel itself.
    pub self_signed: bool,
    /// The common name of the issuer.
    pub issuer: String,
    /// The common name of the subject.
    pub subject: String,
    /// When the certificate expires.
    pub expires_at: Option<String>,
    /// How many days are left.
    pub days_left: Option<i64>,
    /// Whether acme.sh is installed, i.e. whether a real certificate can be
    /// issued at all.
    pub acme_installed: bool,
}

/// Describe the certificate the panel currently serves.
pub fn panel_status(hostname: &str, tls_dir: &Path) -> PanelCertificate {
    let cert_path = tls_dir.join("panel-cert.pem");
    let exists = cert_path.exists();
    let (issuer, subject) = read_names(&cert_path).unwrap_or_default();
    let expires_at = read_expiry(&cert_path);
    let days_left = expires_at.as_deref().and_then(|expiry| {
        chrono::DateTime::parse_from_rfc3339(expiry)
            .ok()
            .map(|expiry| (expiry.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days())
    });
    let self_signed = !issuer.is_empty() && issuer == subject;

    PanelCertificate {
        hostname: hostname.to_string(),
        path: cert_path.to_string_lossy().to_string(),
        exists,
        self_signed,
        issuer,
        subject,
        expires_at,
        days_left,
        acme_installed: AcmePaths::from_env().binary.exists(),
    }
}

/// Issue a certificate for the panel hostname and install it.
///
/// `method` is either `webroot` — the panel serves the ACME challenge from
/// `webroot` — or `standalone`, which lets acme.sh bind port 80 itself.
pub async fn issue_panel(
    hostname: &str,
    tls_dir: &Path,
    method: &str,
    email: Option<&str>,
    webroot: &Path,
) -> Result<IssuanceResult> {
    let acme = AcmePaths::from_env();
    if !acme.binary.exists() {
        bail!(
            "acme.sh is not installed at {}; install it or use a custom certificate",
            acme.binary.display()
        );
    }
    if hostname.trim().is_empty() || hostname == "localhost" {
        bail!("set the panel hostname before requesting a certificate");
    }

    let mut command = Command::new(&acme.binary);
    command.arg("--issue");
    command.arg("--server").arg("letsencrypt");
    command.arg("--domain").arg(hostname);
    match method {
        "standalone" => {
            command.arg("--standalone");
        }
        _ => {
            std::fs::create_dir_all(webroot)
                .with_context(|| format!("cannot create {}", webroot.display()))?;
            command.arg("--webroot").arg(webroot);
        }
    }
    command.arg("--keylength").arg("ec-256");
    command.arg("--force");
    if let Some(email) = email {
        command.arg("--accountemail").arg(email);
    }

    let output = command.output().context("failed to run acme.sh")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        warn!(hostname, %stderr, "acme.sh failed for the panel certificate");
        return Ok(IssuanceResult {
            domain: hostname.to_string(),
            issued: false,
            expires_at: None,
            message: if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            },
        });
    }

    // Install the certificate where the web process expects it.
    let domain_paths = acme.domain_paths(hostname);
    std::fs::create_dir_all(tls_dir)
        .with_context(|| format!("cannot create {}", tls_dir.display()))?;
    let cert_path = tls_dir.join("panel-cert.pem");
    let key_path = tls_dir.join("panel-key.pem");
    std::fs::copy(&domain_paths.fullchain, &cert_path)
        .with_context(|| format!("cannot copy {}", domain_paths.fullchain.display()))?;
    std::fs::copy(&domain_paths.privkey, &key_path)
        .with_context(|| format!("cannot copy {}", domain_paths.privkey.display()))?;
    restrict(&cert_path, 0o644);
    restrict(&key_path, 0o640);
    // The web process runs unprivileged and has to be able to read the key.
    crate::ownership::chown_to_synthpanel(&key_path);

    info!(hostname, "the panel certificate was issued");
    Ok(IssuanceResult {
        domain: hostname.to_string(),
        issued: true,
        expires_at: read_expiry(&cert_path),
        message: "the certificate was issued and installed".to_string(),
    })
}

/// Set the permissions of a file.
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

/// Read the issuer and the subject common name of a PEM certificate.
pub fn read_names(path: &Path) -> Option<(String, String)> {
    let contents = std::fs::read_to_string(path).ok()?;
    let pem = extract_pem(&contents, "CERTIFICATE")?;
    let der = base64_decode(&pem).ok()?;
    let issuer = common_name(&der, 0);
    let subject = common_name(&der, 1);
    Some((issuer, subject))
}

/// The common name of the issuer (`index` 0) or the subject (`index` 1).
///
/// The certificate is walked without a full ASN.1 parser: the names live in
/// the `issuer` and `subject` fields of the `TBSCertificate`.
fn common_name(der: &[u8], index: usize) -> String {
    let mut offset = 0;
    let Some((_, certificate)) = read_der(der, &mut offset) else {
        return String::new();
    };
    let mut inner = 0;
    let Some((_, tbs)) = read_der(certificate, &mut inner) else {
        return String::new();
    };

    // version (optional), serialNumber, signature, issuer, validity, subject
    let mut cursor = 0;
    if let Some((tag, _)) = read_der(tbs, &mut cursor) {
        if tag != 0xA0 {
            cursor = 0;
        }
    }
    let _ = read_der(tbs, &mut cursor); // serialNumber
    let _ = read_der(tbs, &mut cursor); // signature
    let Some((_, issuer)) = read_der(tbs, &mut cursor) else {
        return String::new();
    };
    if index == 0 {
        return attribute(issuer, &[0x55, 0x04, 0x03]);
    }
    let _ = read_der(tbs, &mut cursor); // validity
    let Some((_, subject)) = read_der(tbs, &mut cursor) else {
        return String::new();
    };
    attribute(subject, &[0x55, 0x04, 0x03])
}

/// Find an attribute with the given object identifier and return its value.
///
/// The name is a sequence of sets of sequences, so the search walks into the
/// containers.
fn attribute(data: &[u8], oid: &[u8]) -> String {
    let mut offset = 0;
    while offset < data.len() {
        let Some((tag, content)) = read_der(data, &mut offset) else {
            break;
        };
        match tag {
            0x06 => {
                if content == oid {
                    // The value follows the object identifier.
                    let mut value_offset = offset;
                    if let Some((value_tag, value)) = read_der(data, &mut value_offset) {
                        if matches!(value_tag, 0x0C | 0x13 | 0x16 | 0x1E) {
                            return String::from_utf8_lossy(value).to_string();
                        }
                    }
                }
            }
            0x30 | 0x31 => {
                let found = attribute(content, oid);
                if !found.is_empty() {
                    return found;
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// Read one DER element: its tag and its content.
fn read_der<'a>(data: &'a [u8], offset: &mut usize) -> Option<(u8, &'a [u8])> {
    let tag = *data.get(*offset)?;
    *offset += 1;
    let first = *data.get(*offset)?;
    let length = if first & 0x80 == 0 {
        *offset += 1;
        first as usize
    } else {
        let count = (first & 0x7f) as usize;
        let mut length = 0usize;
        for index in 0..count {
            length = (length << 8) | *data.get(*offset + 1 + index)? as usize;
        }
        *offset += 1 + count;
        length
    };
    let start = *offset;
    let end = start.checked_add(length)?;
    let content = data.get(start..end)?;
    *offset = end;
    Some((tag, content))
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
    // PEM files wrap their base64 payload over several lines.
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned)
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

// ---------------------------------------------------------------------------
// Custom (user-supplied) certificates
// ---------------------------------------------------------------------------

const DEFAULT_SITE_CERT_DIR: &str = "/etc/nginx/synthpanel/certs";
const SITE_CERT_DIR_ENV: &str = "SYNTHPANEL_SITE_CERT_DIR";

/// File layout of a site's custom certificate.
#[derive(Debug, Clone)]
pub struct SiteCertPaths {
    pub dir: PathBuf,
    /// The leaf certificate exactly as supplied.
    pub cert: PathBuf,
    pub key: PathBuf,
    /// The supplied chain, or the leaf certificate when no chain was given so
    /// that `ssl_trusted_certificate` always points at a parseable PEM file.
    pub chain: PathBuf,
    /// The leaf followed by the chain, which is what nginx sends to clients.
    pub fullchain: PathBuf,
}

/// Root directory for per-site custom certificates. Overridable for tests and
/// development containers with `SYNTHPANEL_SITE_CERT_DIR`.
pub fn site_cert_root() -> PathBuf {
    std::env::var_os(SITE_CERT_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SITE_CERT_DIR))
}

pub fn site_cert_paths_in(root: &Path, site_id: &str) -> SiteCertPaths {
    let dir = root.join(site_id);
    SiteCertPaths {
        cert: dir.join("cert.pem"),
        key: dir.join("key.pem"),
        chain: dir.join("chain.pem"),
        fullchain: dir.join("fullchain.pem"),
        dir,
    }
}

/// Paths the `custom` vhost template renders.
pub fn site_cert_paths(site_id: &str) -> SiteCertPaths {
    site_cert_paths_in(&site_cert_root(), site_id)
}

/// Install a user-supplied certificate for a site.
///
/// A missing chain is tolerated: `chain.pem` then contains the leaf so nginx
/// can still parse the file, and `fullchain.pem` contains just the leaf.
pub fn install_custom_certificate(
    site_id: &str,
    cert: &str,
    key: &str,
    chain: Option<&str>,
) -> Result<SiteCertPaths> {
    install_custom_certificate_in(&site_cert_root(), site_id, cert, key, chain)
}

/// Same as [`install_custom_certificate`] with an explicit root directory;
/// useful for tests and for callers that manage their own layout.
pub fn install_custom_certificate_in(
    root: &Path,
    site_id: &str,
    cert: &str,
    key: &str,
    chain: Option<&str>,
) -> Result<SiteCertPaths> {
    validate_site_id(site_id)?;
    let cert = validate_pem("CERTIFICATE", cert, "certificate")?;
    let key = validate_pem("PRIVATE KEY", key, "private key")?;
    let chain = match chain.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(validate_pem("CERTIFICATE", value, "chain")?),
        None => None,
    };

    let paths = site_cert_paths_in(root, site_id);
    std::fs::create_dir_all(&paths.dir)
        .with_context(|| format!("cannot create {}", paths.dir.display()))?;
    restrict(&paths.dir, 0o755);

    let chain_contents = match &chain {
        Some(chain) => format!("{chain}\n"),
        None => format!("# No separate chain supplied; using the leaf certificate.\n{cert}\n"),
    };
    let fullchain = match &chain {
        Some(chain) => format!("{cert}\n{chain}\n"),
        None => format!("{cert}\n"),
    };
    std::fs::write(&paths.cert, format!("{cert}\n"))
        .with_context(|| format!("cannot write {}", paths.cert.display()))?;
    std::fs::write(&paths.chain, chain_contents)
        .with_context(|| format!("cannot write {}", paths.chain.display()))?;
    std::fs::write(&paths.fullchain, fullchain)
        .with_context(|| format!("cannot write {}", paths.fullchain.display()))?;
    std::fs::write(&paths.key, format!("{key}\n"))
        .with_context(|| format!("cannot write {}", paths.key.display()))?;
    restrict(&paths.cert, 0o644);
    restrict(&paths.chain, 0o644);
    restrict(&paths.fullchain, 0o644);
    restrict(&paths.key, 0o600);
    // Best effort: nginx's master reads the key as root, but an unprivileged
    // nginx installation needs the web user to own it.
    crate::ownership::chown_to_synthpanel(&paths.key);
    Ok(paths)
}

fn validate_site_id(site_id: &str) -> Result<()> {
    if site_id.is_empty()
        || !site_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid site id `{site_id}`");
    }
    Ok(())
}

fn validate_pem(label: &str, value: &str, what: &str) -> Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() || !value.contains(&format!("-----BEGIN {label}-----")) {
        bail!("{what} must be a PEM block beginning with -----BEGIN {label}-----");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a self-signed certificate and return its path.
    fn self_signed(dir: &std::path::Path) -> std::path::PathBuf {
        let certified =
            rcgen::generate_simple_self_signed(vec!["panel.example.com".to_string()]).unwrap();
        let path = dir.join("panel-cert.pem");
        std::fs::write(&path, certified.cert.pem()).unwrap();
        path
    }

    #[test]
    fn reads_the_issuer_and_the_subject() {
        let dir = std::env::temp_dir().join(format!("sp-ssl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = self_signed(&dir);
        let (issuer, subject) = read_names(&path).unwrap();
        assert!(!issuer.is_empty(), "the issuer should be readable");
        assert!(!subject.is_empty(), "the subject should be readable");
        assert_eq!(issuer, subject, "the certificate is self-signed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describes_the_panel_certificate() {
        let dir = std::env::temp_dir().join(format!("sp-ssl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let status = panel_status("panel.example.com", &dir);
        assert!(!status.exists);
        assert_eq!(status.hostname, "panel.example.com");

        let path = self_signed(&dir);
        std::fs::write(dir.join("panel-key.pem"), "key").unwrap();
        let status = panel_status("panel.example.com", &dir);
        assert!(status.exists);
        assert!(status.self_signed);
        assert_eq!(status.path, path.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn installs_a_custom_certificate() {
        let dir = std::env::temp_dir().join(format!("sp-ssl-{}", uuid::Uuid::new_v4()));
        let certified =
            rcgen::generate_simple_self_signed(vec!["site.example.com".to_string()]).unwrap();
        let cert = certified.cert.pem();
        let key = certified.signing_key.serialize_pem();
        let chain = "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----";
        let paths =
            install_custom_certificate_in(&dir, "site-123", &cert, &key, Some(chain)).unwrap();

        assert_eq!(paths.dir, dir.join("site-123"));
        assert_eq!(
            std::fs::read_to_string(&paths.cert).unwrap(),
            format!("{}\n", cert.trim())
        );
        assert_eq!(
            std::fs::read_to_string(&paths.key).unwrap(),
            format!("{}\n", key.trim())
        );
        assert!(
            std::fs::read_to_string(&paths.chain)
                .unwrap()
                .contains("QUJD")
        );
        let fullchain = std::fs::read_to_string(&paths.fullchain).unwrap();
        assert!(fullchain.starts_with(cert.trim()));
        assert!(fullchain.contains(chain));

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&paths.key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_chain_still_produces_parseable_certificate_files() {
        let dir = std::env::temp_dir().join(format!("sp-ssl-{}", uuid::Uuid::new_v4()));
        let certified =
            rcgen::generate_simple_self_signed(vec!["site.example.com".to_string()]).unwrap();
        let cert = certified.cert.pem();
        let key = certified.signing_key.serialize_pem();
        let paths = install_custom_certificate_in(&dir, "site-456", &cert, &key, None).unwrap();
        let chain = std::fs::read_to_string(&paths.chain).unwrap();
        assert!(chain.contains("-----BEGIN CERTIFICATE-----"));
        let fullchain = std::fs::read_to_string(&paths.fullchain).unwrap();
        assert_eq!(fullchain.matches("-----BEGIN CERTIFICATE-----").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_certificate_material_and_site_ids() {
        let dir = std::env::temp_dir().join(format!("sp-ssl-{}", uuid::Uuid::new_v4()));
        assert!(install_custom_certificate_in(&dir, "site", "", "key", None).is_err());
        assert!(install_custom_certificate_in(&dir, "../escape", "cert", "key", None).is_err());
        let certified =
            rcgen::generate_simple_self_signed(vec!["site.example.com".to_string()]).unwrap();
        assert!(
            install_custom_certificate_in(&dir, "site", &certified.cert.pem(), "not a key", None)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
