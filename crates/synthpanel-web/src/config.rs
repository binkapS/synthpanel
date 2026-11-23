//! Web process configuration.

use std::env;
use std::path::PathBuf;

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
            assert_eq!(config.ipc_token_path, PathBuf::from("/run/synthpanel/ipc-token"));
            assert_eq!(config.core_socket, PathBuf::from("/run/synthpanel/core.sock"));
        }
    }
}
