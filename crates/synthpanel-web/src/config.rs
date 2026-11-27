//! Web process configuration.

use std::env;
use std::net::IpAddr;
use std::path::PathBuf;

/// An IPv4 or IPv6 network from `SYNTHPANEL_PANEL_ALLOW_IPS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpRange {
    network: IpAddr,
    prefix: u8,
}

impl IpRange {
    /// Parse a bare address (`/32` or `/128` is implied) or a CIDR network.
    /// Anything else is ignored rather than accidentally opening the panel.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let (address, prefix) = match value.split_once('/') {
            Some((address, prefix)) => {
                let prefix = prefix.trim().parse::<u8>().ok()?;
                (address.trim().parse::<IpAddr>().ok()?, prefix)
            }
            None => {
                let address = value.parse::<IpAddr>().ok()?;
                let prefix = match address {
                    IpAddr::V4(_) => 32,
                    IpAddr::V6(_) => 128,
                };
                (address, prefix)
            }
        };
        let bits = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > bits {
            return None;
        }
        Some(Self {
            network: address,
            prefix,
        })
    }

    /// Whether `ip` falls inside this network.
    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - self.prefix)
                };
                u32::from(network) & mask == u32::from(*ip) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - self.prefix)
                };
                u128::from(network) & mask == u128::from(*ip) & mask
            }
            // An IPv4 rule never matches an IPv6 client (and vice versa).
            _ => false,
        }
    }
}

/// Parse the comma-separated `SYNTHPANEL_PANEL_ALLOW_IPS` value.
pub fn parse_ip_list(value: &str) -> Vec<IpRange> {
    value.split(',').filter_map(IpRange::parse).collect()
}

/// Whether an address is allowed through the panel gate. An empty list means
/// that no IP restriction is configured.
pub fn ip_allowed(ranges: &[IpRange], ip: &IpAddr) -> bool {
    ranges.is_empty() || ranges.iter().any(|range| range.contains(ip))
}

/// Credentials for the optional second (HTTP Basic) panel gate.
///
/// The hash is a secret: its `Debug` representation never prints it, and no
/// endpoint returns it.
#[derive(Clone, PartialEq, Eq)]
pub struct PanelBasicAuth {
    pub user: String,
    pub password_hash: String,
}

impl PanelBasicAuth {
    /// A partially configured gate stays on but cannot be passed, so a typo
    /// in one variable cannot silently remove the additional authentication.
    pub fn is_complete(&self) -> bool {
        !self.user.is_empty() && self.password_hash.starts_with("$argon2id$")
    }
}

impl std::fmt::Debug for PanelBasicAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PanelBasicAuth")
            .field("user", &self.user)
            .field("password_hash", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind_addr: String,
    pub port: u16,
    pub core_socket: PathBuf,
    pub ipc_token_path: PathBuf,
    pub tls_dir: PathBuf,
    pub log_dir: PathBuf,
    pub lib_dir: PathBuf,
    /// The document root of the sites.
    pub www_dir: PathBuf,
    pub session_ttl_secs: u64,
    pub secure_cookies: bool,
    pub panel_hostname: String,
    /// Whether the forwarded headers may be trusted.
    pub trusted_proxies: bool,
    /// Optional HTTP Basic credentials in front of the panel.
    pub panel_basic_auth: Option<PanelBasicAuth>,
    /// Optional allow-list of client addresses (CIDRs or bare IPs).
    pub panel_allow_ips: Vec<IpRange>,
}

fn env_str(key: &str, default: &str) -> String {
    match env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => default.to_string(),
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(env_str(key, default))
}

fn env_optional(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl WebConfig {
    pub fn from_env() -> Self {
        let bind_addr = env_str("SYNTHPANEL_WEB_ADDR", "0.0.0.0:8443");
        let port = env_str("SYNTHPANEL_WEB_PORT", "8443")
            .parse()
            .unwrap_or(8443);
        let run_dir = env_path("SYNTHPANEL_RUN_DIR", "/run/synthpanel");
        let core_socket = env_path(
            "SYNTHPANEL_CORE_SOCKET",
            &format!("{}/core.sock", run_dir.display()),
        );
        // Must match the core's default (SYNTHPANEL_RUN_DIR/ipc-token). The
        // run directory is a tmpfs and is not a file-manager scope.
        let ipc_token_path = env_path(
            "SYNTHPANEL_IPC_TOKEN_FILE",
            &format!("{}/ipc-token", run_dir.display()),
        );
        let tls_dir = env_path("SYNTHPANEL_TLS_DIR", "/etc/synthpanel/tls");
        let log_dir = env_path("SYNTHPANEL_LOG_DIR", "/var/log/synthpanel");
        let lib_dir = env_path("SYNTHPANEL_LIB_DIR", "/var/lib/synthpanel");
        let www_dir = env_path("SYNTHPANEL_WWW_DIR", "/var/www");
        let session_ttl_secs = env_str("SYNTHPANEL_SESSION_TTL", "86400")
            .parse()
            .unwrap_or(86400);
        let secure_cookies = env_str("SYNTHPANEL_SECURE_COOKIES", "1") != "0";
        let panel_hostname = env_str("SYNTHPANEL_PANEL_HOSTNAME", "localhost");
        // The forwarded headers are only honoured when the panel is told that
        // it sits behind a proxy it trusts.
        let trusted_proxies = !env_str("SYNTHPANEL_TRUSTED_PROXY", "").is_empty();
        let basic_user = env_optional("SYNTHPANEL_PANEL_BASIC_USER");
        let basic_hash = env_optional("SYNTHPANEL_PANEL_BASIC_PASSWORD_HASH");
        let panel_basic_auth = if basic_user.is_none() && basic_hash.is_none() {
            None
        } else {
            // Keep the gate enabled even when only one half is configured:
            // `is_complete()` then makes every request fail closed.
            Some(PanelBasicAuth {
                user: basic_user.unwrap_or_default(),
                password_hash: basic_hash.unwrap_or_default(),
            })
        };
        let panel_allow_ips = env::var("SYNTHPANEL_PANEL_ALLOW_IPS")
            .map(|value| parse_ip_list(&value))
            .unwrap_or_default();

        Self {
            bind_addr,
            port,
            core_socket,
            ipc_token_path,
            tls_dir,
            log_dir,
            lib_dir,
            www_dir,
            session_ttl_secs,
            secure_cookies,
            panel_hostname,
            trusted_proxies,
            panel_basic_auth,
            panel_allow_ips,
        }
    }

    /// Whether the panel may trust `X-Forwarded-For` and `X-Forwarded-Proto`.
    ///
    /// The headers are only honoured when the operator says the panel runs
    /// behind a proxy; otherwise a client could forge its own address in the
    /// audit log.
    pub fn trusts_proxies(&self) -> bool {
        self.trusted_proxies
    }

    /// Whether an IP allow-list is configured.
    pub fn ip_allow_list_enabled(&self) -> bool {
        !self.panel_allow_ips.is_empty()
    }

    /// Whether `ip` may reach the panel.
    pub fn allows_ip(&self, ip: &IpAddr) -> bool {
        ip_allowed(&self.panel_allow_ips, ip)
    }

    /// The directory that holds the ACME challenges for the panel hostname.
    pub fn acme_webroot(&self) -> PathBuf {
        self.lib_dir.join("acme-webroot")
    }

    pub fn tls_cert_path(&self) -> PathBuf {
        self.tls_dir.join("panel-cert.pem")
    }

    pub fn tls_key_path(&self) -> PathBuf {
        self.tls_dir.join("panel-key.pem")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = WebConfig::from_env();
        assert_eq!(config.port, 8443);
        assert_eq!(config.session_ttl_secs, 86400);
        assert!(config.secure_cookies);
        // The forwarded headers are not trusted unless the panel is told so.
        assert!(!config.trusts_proxies());
        if std::env::var_os("SYNTHPANEL_IPC_TOKEN_FILE").is_none() {
            // Must point at the same tmpfs token the core writes.
            assert_eq!(
                config.ipc_token_path,
                PathBuf::from("/run/synthpanel/ipc-token")
            );
            assert_eq!(
                config.core_socket,
                PathBuf::from("/run/synthpanel/core.sock")
            );
        }
    }

    #[test]
    fn bare_addresses_are_exact_host_routes() {
        let v4 = IpRange::parse("10.0.0.7").unwrap();
        assert!(v4.contains(&"10.0.0.7".parse().unwrap()));
        assert!(!v4.contains(&"10.0.0.8".parse().unwrap()));
        let v6 = IpRange::parse("2001:db8::7").unwrap();
        assert!(v6.contains(&"2001:db8::7".parse().unwrap()));
        assert!(!v6.contains(&"2001:db8::8".parse().unwrap()));
    }

    #[test]
    fn cidrs_match_the_whole_network() {
        let cases = [
            ("10.0.0.0/8", "10.1.2.3", true),
            ("10.0.0.0/8", "11.1.2.3", false),
            ("10.0.0.0/0", "192.0.2.1", true),
            ("192.0.2.0/24", "192.0.2.255", true),
            ("192.0.2.0/24", "192.0.3.1", false),
            ("192.0.2.7/32", "192.0.2.7", true),
            ("192.0.2.7/32", "192.0.2.8", false),
            ("2001:db8::/32", "2001:db8:abcd::1", true),
            ("2001:db8::/32", "2001:db9::1", false),
            ("::/0", "2001:db8::1", true),
        ];
        for (range, address, expected) in cases {
            let range = IpRange::parse(range).expect("valid range");
            let address: IpAddr = address.parse().unwrap();
            assert_eq!(range.contains(&address), expected, "{range:?} vs {address}");
        }
    }

    #[test]
    fn address_families_never_cross() {
        let v4 = IpRange::parse("0.0.0.0/0").unwrap();
        let v6 = IpRange::parse("::/0").unwrap();
        assert!(!v4.contains(&"2001:db8::1".parse().unwrap()));
        assert!(!v6.contains(&"192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn malformed_ranges_are_rejected() {
        for value in [
            "",
            " ",
            "not-an-ip",
            "10.0.0.1/33",
            "2001:db8::1/129",
            "10.0.0.1/-1",
            "10.0.0.1/abc",
            "10.0.0.1/24/8",
            "10.0.0.256/24",
        ] {
            assert!(IpRange::parse(value).is_none(), "accepted `{value}`");
        }
    }

    #[test]
    fn an_empty_allow_list_admits_everyone() {
        let ip: IpAddr = "203.0.113.4".parse().unwrap();
        assert!(ip_allowed(&[], &ip));
        let ranges = parse_ip_list("198.51.100.0/24, 203.0.113.4");
        assert!(ip_allowed(&ranges, &ip));
        assert!(!ip_allowed(&ranges, &"203.0.113.5".parse().unwrap()));
        assert!(ip_allowed(&ranges, &"198.51.100.99".parse().unwrap()));
    }

    #[test]
    fn invalid_entries_do_not_disable_the_valid_ones() {
        let ranges = parse_ip_list("garbage, 192.0.2.0/24, 10.0.0.1/99,");
        assert_eq!(ranges.len(), 1);
        assert!(ip_allowed(&ranges, &"192.0.2.9".parse().unwrap()));
        assert!(!ip_allowed(&ranges, &"10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn a_half_configured_basic_gate_fails_closed() {
        let complete = PanelBasicAuth {
            user: "admin".into(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRz$abc".into(),
        };
        assert!(complete.is_complete());
        let missing_user = PanelBasicAuth {
            user: "".into(),
            ..complete.clone()
        };
        assert!(!missing_user.is_complete());
        let wrong_algorithm = PanelBasicAuth {
            user: "admin".into(),
            password_hash: "$argon2i$v=19$m=19456,t=2,p=1$c29tZXNhbHRz$abc".into(),
        };
        assert!(!wrong_algorithm.is_complete());
        // Debug output must not carry the password hash.
        assert!(!format!("{complete:?}").contains("argon2id"));
    }
}
