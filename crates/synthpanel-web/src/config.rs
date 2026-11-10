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
    pub static_dir: PathBuf,
    pub session_ttl_secs: u64,
    pub secure_cookies: bool,
    pub panel_hostname: String,
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
        let core_socket = env_path("SYNTHPANEL_CORE_SOCKET", "/run/synthpanel/core.sock");
        let ipc_token_path = env_path("SYNTHPANEL_IPC_TOKEN_FILE", "/etc/synthpanel/ipc-token");
        let tls_dir = env_path("SYNTHPANEL_TLS_DIR", "/etc/synthpanel/tls");
        let log_dir = env_path("SYNTHPANEL_LOG_DIR", "/var/log/synthpanel");
        let static_dir = env_path("SYNTHPANEL_STATIC_DIR", "frontend/dist");
        let session_ttl_secs = env_str("SYNTHPANEL_SESSION_TTL", "86400")
            .parse()
            .unwrap_or(86400);
        let secure_cookies = env_str("SYNTHPANEL_SECURE_COOKIES", "1") != "0";
        let panel_hostname = env_str("SYNTHPANEL_PANEL_HOSTNAME", "localhost");

        Self {
            bind_addr,
            port,
            core_socket,
            ipc_token_path,
            tls_dir,
            log_dir,
            static_dir,
            session_ttl_secs,
            secure_cookies,
            panel_hostname,
        }
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
    }
}
