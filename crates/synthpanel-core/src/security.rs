//! Host security: firewall rules, fail2ban jails and SSH keys.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// A firewall rule as the panel models it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallRule {
    pub port: String,
    pub proto: String,
    pub source: String,
    pub action: String,
    pub comment: String,
    pub enabled: bool,
}

impl FirewallRule {
    /// The `ufw` arguments for this rule.
    pub fn ufw_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        match self.action.as_str() {
            "allow" => args.push("allow".into()),
            "deny" => args.push("deny".into()),
            "reject" => args.push("reject".into()),
            "limit" => args.push("limit".into()),
            other => args.push(other.to_string()),
        }
        if self.port == "any" || self.port.is_empty() {
            // A rule without a port applies to every port.
        } else {
            args.push(format!("{}/{}", self.port, self.proto));
        }
        if self.source != "any" && !self.source.is_empty() {
            args.push("from".into());
            args.push(self.source.clone());
        }
        if !self.comment.is_empty() {
            args.push("comment".into());
            args.push(format!("'{}'", self.comment.replace('\'', "")));
        }
        args
    }
}

/// Validate a port or port range.
pub fn validate_port(port: &str) -> Result<String> {
    let port = port.trim();
    if port.is_empty() {
        bail!("the port must not be empty");
    }
    if port == "any" {
        return Ok(port.to_string());
    }
    for part in port.split(':') {
        let number: u32 = part
            .parse()
            .with_context(|| format!("`{part}` is not a valid port"))?;
        if number == 0 || number > 65535 {
            bail!("the port must be between 1 and 65535");
        }
    }
    Ok(port.to_string())
}

/// Validate a source address or CIDR range.
pub fn validate_source(source: &str) -> Result<String> {
    let source = source.trim();
    if source.is_empty() || source == "any" {
        return Ok("any".to_string());
    }
    if source.contains('/') {
        let (address, bits) = source
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("invalid CIDR range"))?;
        let bits: u32 = bits
            .parse()
            .with_context(|| format!("`{bits}` is not a valid CIDR prefix length"))?;
        if bits > 32 {
            bail!("the CIDR prefix length must be between 0 and 32");
        }
        if address.parse::<std::net::Ipv4Addr>().is_err() {
            bail!("`{address}` is not a valid IPv4 address");
        }
        return Ok(source.to_string());
    }
    if source.parse::<std::net::IpAddr>().is_err() {
        bail!("`{source}` is not a valid IP address");
    }
    Ok(source.to_string())
}

/// Run `ufw` and return its output.
pub fn ufw(args: &[&str]) -> Result<String> {
    let output = Command::new("ufw")
        .args(args)
        .output()
        .context("cannot run ufw; is it installed?")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ufw failed: {}{}", stderr.trim(), stdout.trim());
    }
    Ok(stdout.to_string())
}

/// List the current firewall rules.
pub fn ufw_status() -> Result<String> {
    ufw(&["status", "numbered"])
}

/// Apply a rule to the firewall.
pub fn ufw_apply(rule: &FirewallRule) -> Result<String> {
    if !rule.enabled {
        return Ok("skipped (rule disabled)".into());
    }
    let args = rule.ufw_args();
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    ufw(&borrowed)
}

/// Remove a rule by its `ufw` index.
pub fn ufw_delete(index: u32) -> Result<String> {
    ufw(&["--force", "delete", &index.to_string()])
}

/// Whether ufw is installed.
pub fn ufw_available() -> bool {
    Command::new("ufw").arg("version").output().is_ok()
}

/// A fail2ban jail and its current state.
#[derive(Debug, Clone, Serialize)]
pub struct JailStatus {
    pub name: String,
    pub status: String,
    pub currently_banned: u32,
    pub total_banned: u32,
}

/// List the fail2ban jails.
pub fn fail2ban_jails() -> Result<Vec<JailStatus>> {
    let output = Command::new("fail2ban-client")
        .arg("status")
        .output()
        .context("cannot run fail2ban-client; is fail2ban installed?")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut jails = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(list) = line.strip_prefix("`- ") {
            // `Jail list: sshd, nginx-http-auth`
            let names = list
                .split_once(':')
                .map(|(_, value)| value)
                .unwrap_or(list)
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty());
            for name in names {
                jails.push(JailStatus {
                    name: name.to_string(),
                    status: "unknown".into(),
                    currently_banned: 0,
                    total_banned: 0,
                });
            }
        }
    }
    for jail in &mut jails {
        if let Ok(status) = fail2ban_jail_status(&jail.name) {
            jail.status = status.0;
            jail.currently_banned = status.1;
            jail.total_banned = status.2;
        }
    }
    Ok(jails)
}

/// Fetch the status of a single jail: (status, currently banned, total banned).
pub fn fail2ban_jail_status(name: &str) -> Result<(String, u32, u32)> {
    let output = Command::new("fail2ban-client")
        .arg("status")
        .arg(name)
        .output()
        .context("cannot run fail2ban-client")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        bail!("fail2ban-client failed: {}", stdout.trim());
    }
    let status = stdout
        .lines()
        .find(|line| line.contains("Status for the jail"))
        .and_then(|line| line.split('|').nth(1))
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let currently = stdout
        .lines()
        .find(|line| line.contains("Currently banned"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let total = stdout
        .lines()
        .find(|line| line.contains("Total banned"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    Ok((status, currently, total))
}

/// Unban an address from a jail.
pub fn fail2ban_unban(jail: &str, ip: &str) -> Result<String> {
    let ip = validate_source(ip)?;
    let output = Command::new("fail2ban-client")
        .arg("set")
        .arg(jail)
        .arg("unbanip")
        .arg(&ip)
        .output()
        .context("cannot run fail2ban-client")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("fail2ban-client failed: {}{}", stderr.trim(), stdout.trim());
    }
    Ok(stdout.trim().to_string())
}

/// Whether fail2ban is installed.
pub fn fail2ban_available() -> bool {
    Command::new("fail2ban-client")
        .arg("--version")
        .output()
        .is_ok()
}

/// An SSH public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKey {
    pub id: String,
    pub label: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: String,
}

/// Validate an SSH public key and return its type and fingerprint.
pub fn validate_ssh_key(key: &str) -> Result<(String, String)> {
    let key = key.trim();
    let parts: Vec<&str> = key.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("the key must contain a type and the key material");
    }
    let key_type = parts[0].to_string();
    if !matches!(
        key_type.as_str(),
        "ssh-rsa"
            | "ssh-ed25519"
            | "ssh-dss"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "sk-ssh-ed25519@openssh.com"
            | "sk-ecdsa-sha2-nistp256@openssh.com"
    ) {
        bail!("unsupported key type `{key_type}`");
    }
    if !parts[1]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        bail!("the key material is not valid base64");
    }
    Ok((key_type, fingerprint(key)))
}

/// Compute the OpenSSH-style `SHA256:...` fingerprint of a key.
pub fn fingerprint(key: &str) -> String {
    use sha2::{Digest, Sha256};
    let parts: Vec<&str> = key.split_whitespace().collect();
    if parts.len() < 2 {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(parts[1].as_bytes());
    let digest = hasher.finalize();
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
    format!("SHA256:{encoded}")
}

/// The file that holds the authorised keys for a user.
pub fn authorized_keys_path(user: &str) -> std::path::PathBuf {
    let home = match user {
        "root" => "/root".to_string(),
        other => format!("/home/{other}"),
    };
    std::path::PathBuf::from(home)
        .join(".ssh")
        .join("authorized_keys")
}

/// Add a key to a user's authorised keys file.
pub fn install_ssh_key(user: &str, key: &str) -> Result<()> {
    let path = authorized_keys_path(user);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut contents = std::fs::read_to_string(&path).unwrap_or_default();
    let trimmed = key.trim();
    if !contents.lines().any(|line| line.trim() == trimmed) {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(trimmed);
        contents.push('\n');
        std::fs::write(&path, contents)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    if let Some(parent) = path.parent() {
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Remove a key from a user's authorised keys file.
pub fn remove_ssh_key(user: &str, key: &str) -> Result<bool> {
    let path = authorized_keys_path(user);
    if !path.exists() {
        return Ok(false);
    }
    let contents = std::fs::read_to_string(&path)?;
    let trimmed = key.trim();
    let kept: Vec<&str> = contents
        .lines()
        .filter(|line| line.trim() != trimmed)
        .collect();
    let removed = kept.len() != contents.lines().count();
    let mut updated = kept.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    std::fs::write(&path, updated)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_ufw_arguments() {
        let rule = FirewallRule {
            port: "443".into(),
            proto: "tcp".into(),
            source: "any".into(),
            action: "allow".into(),
            comment: "https".into(),
            enabled: true,
        };
        let args = rule.ufw_args();
        assert_eq!(args[0], "allow");
        assert!(args.contains(&"443/tcp".to_string()));
        assert!(!args.contains(&"from".to_string()));
    }

    #[test]
    fn builds_ufw_arguments_with_a_source() {
        let rule = FirewallRule {
            port: "8443".into(),
            proto: "tcp".into(),
            source: "10.0.0.0/8".into(),
            action: "allow".into(),
            comment: String::new(),
            enabled: true,
        };
        let args = rule.ufw_args();
        assert!(args.contains(&"from".to_string()));
        assert!(args.contains(&"10.0.0.0/8".to_string()));
    }

    #[test]
    fn validates_ports() {
        assert_eq!(validate_port("80").unwrap(), "80");
        assert_eq!(validate_port("8000:8080").unwrap(), "8000:8080");
        assert!(validate_port("0").is_err());
        assert!(validate_port("70000").is_err());
        assert!(validate_port("http").is_err());
    }

    #[test]
    fn validates_sources() {
        assert_eq!(validate_source("any").unwrap(), "any");
        assert_eq!(validate_source("10.0.0.1").unwrap(), "10.0.0.1");
        assert_eq!(validate_source("10.0.0.0/8").unwrap(), "10.0.0.0/8");
        assert!(validate_source("999.1.1.1").is_err());
        assert!(validate_source("10.0.0.0/99").is_err());
    }

    #[test]
    fn validates_ssh_keys() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyMaterial user@host";
        let (kind, print) = validate_ssh_key(key).unwrap();
        assert_eq!(kind, "ssh-ed25519");
        assert!(print.starts_with("SHA256:"));
        assert!(validate_ssh_key("not-a-key").is_err());
        assert!(validate_ssh_key("bogus-type AAAA").is_err());
    }

    #[test]
    fn fingerprint_is_stable() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyMaterial user@host";
        assert_eq!(fingerprint(key), fingerprint(key));
    }
}
