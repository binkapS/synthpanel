//!Filesystem layout and environment-driven overrides.
//!
//! Production paths follow the plan (§12). Every path can be overridden with a
//! `SYNTHPANEL_*` environment variable so the daemon can be exercised in
//! development containers and tests without root.

use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub etc_dir: PathBuf,
    pub lib_dir: PathBuf,
    pub log_dir: PathBuf,
    pub run_dir: PathBuf,
    pub www_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub ipc_token_path: PathBuf,
    pub tls_dir: PathBuf,
    pub secret_key_path: PathBuf,
}

fn env_path(key: &str, default: &str) -> PathBuf {
    match env::var_os(key) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(default),
    }
}

impl Paths {
    pub fn from_env() -> Self {
        let etc_dir = env_path("SYNTHPANEL_ETC_DIR", "/etc/synthpanel");
        let lib_dir = env_path("SYNTHPANEL_LIB_DIR", "/var/lib/synthpanel");
        let log_dir = env_path("SYNTHPANEL_LOG_DIR", "/var/log/synthpanel");
        let run_dir = env_path("SYNTHPANEL_RUN_DIR", "/run/synthpanel");
        let www_dir = env_path("SYNTHPANEL_WWW_DIR", "/var/www");
        let backup_dir = env_path("SYNTHPANEL_BACKUP_DIR", "/var/backups/synthpanel");

        let db_path = env_path(
            "SYNTHPANEL_DB_PATH",
            &format!("{}/synthpanel.db", lib_dir.display()),
        );
        let socket_path = env_path(
            "SYNTHPANEL_SOCKET_PATH",
            &format!("{}/core.sock", run_dir.display()),
        );
        let ipc_token_path = env_path(
            "SYNTHPANEL_IPC_TOKEN_FILE",
            &format!("{}/ipc-token", etc_dir.display()),
        );
        let tls_dir = env_path("SYNTHPANEL_TLS_DIR", &format!("{}/tls", etc_dir.display()));
        let secret_key_path = env_path(
            "SYNTHPANEL_SECRET_KEY_FILE",
            &format!("{}/secret.key", etc_dir.display()),
        );

        Self {
            etc_dir,
            lib_dir,
            log_dir,
            run_dir,
            www_dir,
            backup_dir,
            db_path,
            socket_path,
            ipc_token_path,
            tls_dir,
            secret_key_path,
        }
    }

    /// Create all directories the daemon needs. Best-effort: errors are
    /// returned so the caller can decide whether to abort.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [
            &self.etc_dir,
            &self.lib_dir,
            &self.log_dir,
            &self.run_dir,
            &self.www_dir,
            &self.backup_dir,
            &self.tls_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_absolute() {
        // Clear overrides so the test is deterministic.
        for key in [
            "SYNTHPANEL_ETC_DIR",
            "SYNTHPANEL_LIB_DIR",
            "SYNTHPANEL_LOG_DIR",
            "SYNTHPANEL_RUN_DIR",
            "SYNTHPANEL_WWW_DIR",
            "SYNTHPANEL_BACKUP_DIR",
            "SYNTHPANEL_DB_PATH",
            "SYNTHPANEL_SOCKET_PATH",
            "SYNTHPANEL_IPC_TOKEN_FILE",
            "SYNTHPANEL_TLS_DIR",
            "SYNTHPANEL_SECRET_KEY_FILE",
        ] {
            unsafe { std::env::remove_var(key) };
        }
        let paths = Paths::from_env();
        assert!(paths.etc_dir.is_absolute());
        assert!(paths.db_path.is_absolute());
        assert!(paths.socket_path.is_absolute());
    }

    #[test]
    fn env_overrides_win() {
        unsafe {
            std::env::set_var("SYNTHPANEL_ETC_DIR", "/tmp/sp-etc");
            std::env::set_var("SYNTHPANEL_LIB_DIR", "/tmp/sp-lib");
        }
        let paths = Paths::from_env();
        assert_eq!(paths.etc_dir, PathBuf::from("/tmp/sp-etc"));
        assert_eq!(paths.lib_dir, PathBuf::from("/tmp/sp-lib"));
        unsafe {
            std::env::remove_var("SYNTHPANEL_ETC_DIR");
            std::env::remove_var("SYNTHPANEL_LIB_DIR");
        }
    }
}
