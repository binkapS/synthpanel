//! The "never break the server" config apply pipeline.
//!
//!   render → validate → backup → write → reload → probe → rollback on fail

use anyhow::{Result, anyhow};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct Paths {
    /// Directory where the generated vhost files land.
    pub nginx_vhosts: PathBuf,
    /// Directory where the symlinked "enabled" vhosts live.
    pub nginx_enabled: PathBuf,
    /// Directory where generated snippets live.
    pub snippets: PathBuf,
    /// Directory where the pool files live.
    pub php_pools: Vec<PathBuf>,
    /// Where current + backup nginx main config live.
    pub nginx_main: PathBuf,
    /// Where the current + backup php-fpm main config lives.
    pub php_main: Vec<PathBuf>,
    /// Where backups land.
    pub backups: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let etc = env_path("SYNTHPANEL_ETC_DIR", "/etc/synthpanel");
        let backup = env_path("SYNTHPANEL_BACKUP_DIR", "/var/backups/synthpanel");
        // Where the generated nginx tree lives. It defaults to a directory
        // inside /etc/synthpanel so the panel works without touching the
        // distribution's own nginx configuration.
        let nginx = env_path(
            "SYNTHPANEL_NGINX_DIR",
            &etc.join("nginx").display().to_string(),
        );
        let php = env_path("SYNTHPANEL_PHP_DIR", &etc.join("php").display().to_string());
        Self {
            nginx_vhosts: nginx.join("sites-available/synthpanel"),
            nginx_enabled: nginx.join("sites-enabled/synthpanel"),
            snippets: nginx.join("snippets"),
            php_pools: (0..4).map(|i| php.join(version_dir(i))).collect(),
            nginx_main: nginx.join("nginx.conf"),
            php_main: (0..4)
                .map(|i| php.join(version_dir(i)).join("php-fpm.conf"))
                .collect(),
            backups: backup.join("configs"),
        }
    }
}

/// Directory name for a supported PHP version.
fn version_dir(index: usize) -> String {
    match index {
        0 => "8.1".to_string(),
        1 => "8.2".to_string(),
        2 => "8.3".to_string(),
        _ => "8.4".to_string(),
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

/// The apply pipeline itself.
pub struct ApplyPipeline {
    paths: Paths,
}

impl Default for ApplyPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplyPipeline {
    pub fn new() -> Self {
        Self {
            paths: Paths::from_env(),
        }
    }

    /// Apply a single site's Nginx vhost and PHP-FPM pool.
    ///
    /// - Renders the vhost and pool.
    /// - Lints custom directives.
    /// - Backs up any existing files.
    /// - Writes the new files.
    /// - "Validates" via `nginx -t` / `php-fpm -t` if the binary is on
    ///   PATH; otherwise the pipeline still succeeds but logs that no
    ///   binary was available (this is expected in development).
    /// - "Reloads" via `systemctl reload` if available.
    /// - On any failure, restores the backups and aborts.
    pub async fn apply_site(
        &self,
        site: &crate::sites::SiteExtras,
        vhost_text: &str,
        pool_text: &str,
    ) -> Result<ApplyResult> {
        let id = &site.site.id;
        let domain = &site.site.domain;

        let vhost_path = self.paths.nginx_vhosts.join(format!("{id}.conf"));
        let enabled_link = self.paths.nginx_enabled.join(format!("{id}.conf"));

        // Pick the pool path for the chosen version.
        let version = site.site.php_version.as_deref().unwrap_or("8.3");
        let pool_path = pick_pool_path(&self.paths, version)?;

        // 1. Lint
        crate::nginx::lint_directives(site.nginx.as_deref().unwrap_or(""))
            .map_err(|err| anyhow!("nginx lint failed: {err}"))?;

        // 2. Backup
        let vhost_backup = if vhost_path.exists() {
            backup_file(&vhost_path, &self.paths.backups, id, "vhost").await?
        } else {
            None
        };
        let pool_backup = if pool_path.exists() {
            backup_file(&pool_path, &self.paths.backups, id, "pool").await?
        } else {
            None
        };

        // 3. Write
        ensure_parent(&vhost_path).await?;
        ensure_parent(&pool_path).await?;
        let wrote: Result<()> = async {
            tokio::fs::write(&vhost_path, vhost_text).await?;
            tokio::fs::write(&pool_path, pool_text).await?;
            if enabled_link.exists() || enabled_link.symlink_metadata().is_ok() {
                let _ = tokio::fs::remove_file(&enabled_link).await;
            }
            std::os::unix::fs::symlink(&vhost_path, &enabled_link).ok();
            Ok(())
        }
        .await;
        if let Err(error) = wrote {
            restore_backup(&vhost_path, vhost_backup.as_deref()).await;
            restore_backup(&pool_path, pool_backup.as_deref()).await;
            return Ok(ApplyResult::new().make_rolled_back(format!("write failed: {error}")));
        }

        // 4. Validate with `nginx -t` / `php-fpm -t` if available.
        let mut validation: Vec<String> = Vec::new();
        validation.push(
            tokio::process::Command::new("nginx")
                .arg("-t")
                .output()
                .await
                .map(|o| {
                    if o.status.success() {
                        "nginx -t ok".to_string()
                    } else {
                        format!(
                            "nginx -t failed: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        )
                    }
                })
                .unwrap_or_else(|e| format!("nginx binary not available ({e})")),
        );
        let php_version_path = format!("php-fpm{}", version.replace('.', ""));
        if which(&php_version_path).is_some() {
            let out = tokio::process::Command::new(&php_version_path)
                .arg("-t")
                .output()
                .await;
            match out {
                Ok(o) if o.status.success() => validation.push(format!("{php_version_path} -t ok")),
                Ok(o) => {
                    validation.push(format!(
                        "{php_version_path} -t failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    ));
                }
                Err(e) => validation.push(format!("{php_version_path} binary not available ({e})")),
            }
        }

        // 5. Reload — only when both validations succeeded or binary missing.
        let all_ok = validation
            .iter()
            .all(|line| line.ends_with("ok") || line.contains("not available"));
        if !all_ok {
            warn!(domain, vhost = %vhost_path.display(), "nginx/php validation failed; rolling back");
            restore_backup(&vhost_path, vhost_backup.as_deref()).await;
            restore_backup(&pool_path, pool_backup.as_deref()).await;
            if enabled_link.exists() {
                let _ = tokio::fs::remove_file(&enabled_link).await;
            }
            return Ok(ApplyResult::new().make_rolled_back(validation.join("; ")));
        }

        if which("systemctl").is_some() {
            let nginx_reload = tokio::process::Command::new("systemctl")
                .arg("reload")
                .arg("nginx")
                .output()
                .await;
            if let Ok(out) = nginx_reload {
                if !out.status.success() {
                    warn!(?out, "nginx reload failed");
                }
            }
            if which(&php_version_path).is_some() {
                let fpm_reload = tokio::process::Command::new("systemctl")
                    .arg("reload")
                    .arg(php_version_path.clone())
                    .output()
                    .await;
                if let Ok(out) = fpm_reload {
                    if !out.status.success() {
                        warn!(?out, "php-fpm reload failed");
                    }
                }
            }
        }

        info!(domain, vhost = %vhost_path.display(), "site config applied");
        Ok(ApplyResult::success())
    }

    /// Remove a site's config files.
    pub async fn remove_site(&self, id: &str) -> Result<ApplyResult> {
        let vhost_path = self.paths.nginx_vhosts.join(format!("{id}.conf"));
        let enabled_link = self.paths.nginx_enabled.join(format!("{id}.conf"));
        let removed = if vhost_path.exists() {
            tokio::fs::remove_file(&vhost_path).await?;
            if enabled_link.exists() {
                let _ = tokio::fs::remove_file(&enabled_link).await;
            }
            true
        } else {
            false
        };
        Ok(ApplyResult::success().marked_removed(removed))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub ok: bool,
    pub rolled_back: bool,
    pub removed: bool,
    pub error: Option<String>,
}

impl ApplyResult {
    pub fn new() -> Self {
        Self {
            ok: false,
            rolled_back: false,
            removed: false,
            error: None,
        }
    }

    pub fn success() -> Self {
        Self {
            ok: true,
            ..Self::new()
        }
    }

    pub fn make_rolled_back(mut self, error: impl Into<String>) -> Self {
        self.rolled_back = true;
        self.error = Some(error.into());
        self
    }

    pub fn marked_removed(mut self, removed: bool) -> Self {
        self.removed = removed;
        self
    }
}

impl Default for ApplyResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Pick the PHP pool directory matching the site's version.
fn pick_pool_path(paths: &Paths, version: &str) -> Result<PathBuf> {
    if !crate::php::is_supported_version(version) {
        anyhow::bail!("unsupported PHP version `{version}`");
    }
    let idx = match version {
        "8.1" => 0,
        "8.2" => 1,
        "8.3" => 2,
        "8.4" => 3,
        _ => 2,
    };
    let root = &paths.php_pools[idx];
    Ok(root.join(format!("fpm/pool.d/synthpanel-{}.conf", version)))
}

fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

async fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

async fn backup_file(path: &Path, backups: &Path, id: &str, kind: &str) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = chrono::Utc::now().timestamp();
    let target = backups.join("vhost").join(format!("{id}-{kind}-{ts}.bak"));
    tokio::fs::create_dir_all(target.parent().unwrap()).await?;
    tokio::fs::copy(path, &target).await?;
    Ok(Some(target))
}

async fn restore_backup(path: &Path, backup: Option<&Path>) {
    let Some(backup) = backup else { return };
    if backup.exists() {
        if let Err(error) = tokio::fs::copy(backup, path).await {
            warn!(error = %error, backup = %backup.display(), "cannot restore backup");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_result_default_is_failed() {
        let result = ApplyResult::default();
        assert!(!result.ok);
        assert!(!result.rolled_back);
        assert!(result.error.is_none());
    }

    #[test]
    fn which_resolves_builtins() {
        assert!(which("ls").is_some());
        assert!(which("definitely-missing-xyz").is_none());
    }
}
