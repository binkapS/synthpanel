//! Node.js / Python application configuration and process supervision.
//!
//! A site with a row in `site_apps` is served by a local application process
//! instead of by the static/PHP vhost. The core keeps the process alive with a
//! per-site systemd service where systemd is available; otherwise it falls
//! back to a detached `nohup` process under `SYNTHPANEL_RUN_DIR`.
//!
//! In both modes the application runs as the site's pool user (`sp_<id>`).
//! This module refuses to start anything as root, even though the core itself
//! is privileged. All external binaries can be overridden with environment
//! variables so tests and containers can substitute fakes:
//! `SYNTHPANEL_SYSTEMCTL_BIN`, `SYNTHPANEL_NOHUP_BIN`, `SYNTHPANEL_SH_BIN`
//! and `SYNTHPANEL_SYSTEMD_UNIT_DIR`.

use crate::config::Paths;
use crate::sites::SiteRecord;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use std::ffi::CString;
use std::fs::{self, File, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::Command;

/// A validated application type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Node,
    Python,
}

impl AppType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }
}

impl std::str::FromStr for AppType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "node" | "nodejs" => Ok(Self::Node),
            "python" | "python3" => Ok(Self::Python),
            other => Err(format!(
                "unsupported app type `{other}`; expected `node` or `python`"
            )),
        }
    }
}

impl std::fmt::Display for AppType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stored configuration of a site's application.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SiteApp {
    pub site_id: String,
    pub app_type: String,
    pub app_port: i64,
    pub app_command: String,
    pub updated_at: String,
}

impl SiteApp {
    /// The validated type, re-checked on read (the database enforces it too).
    pub fn kind(&self) -> Result<AppType> {
        self.app_type.parse::<AppType>().map_err(anyhow::Error::msg)
    }

    /// The upstream description consumed by the nginx renderer.
    pub fn proxy(&self) -> crate::nginx::AppProxy<'_> {
        crate::nginx::AppProxy::new(
            &self.app_type,
            self.app_port.clamp(0, u16::MAX as i64) as u16,
        )
    }
}

/// The result of `sites.app.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppStatus {
    pub app_type: String,
    pub app_port: i64,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
}

/// Parse and validate an app type parameter.
pub fn parse_app_type(value: &str) -> Result<AppType> {
    value
        .parse::<AppType>()
        .map_err(|error| anyhow::anyhow!(error))
}

/// Validate a TCP port, returning it as `u16` for command construction.
pub fn validate_app_port(port: i64) -> Result<u16> {
    if !(1..=65535).contains(&port) {
        bail!("app_port must be between 1 and 65535, got {port}");
    }
    Ok(port as u16)
}

/// Reject commands that cannot be represented in a unit/script file.
pub fn validate_app_command(command: &str) -> Result<()> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        bail!("app_command must not be empty");
    }
    if command.chars().any(|c| matches!(c, '\n' | '\r' | '\0')) {
        bail!("app_command must be a single line");
    }
    if trimmed
        .split_whitespace()
        .next()
        .is_some_and(|c| c == "sudo")
    {
        bail!("app_command must not use sudo; the app runs as the site user");
    }
    Ok(())
}

/// Read a site's stored app configuration. `None` means a plain site.
pub async fn get_app(pool: &SqlitePool, site_id: &str) -> Result<Option<SiteApp>> {
    let row = sqlx::query_as::<_, SiteApp>("SELECT * FROM site_apps WHERE site_id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Store (or replace) the app configuration of an existing site.
///
/// This does not touch the process or the vhost; use [`configure`] for the
/// full operation, or call this from a handler that applies the vhost itself.
pub async fn configure_app(
    pool: &SqlitePool,
    site_id: &str,
    app_type: &str,
    app_port: i64,
    app_command: &str,
) -> Result<SiteApp> {
    if site_id.trim().is_empty() {
        bail!("site_id must not be empty");
    }
    let kind = parse_app_type(app_type)?;
    validate_app_port(app_port)?;
    validate_app_command(app_command)?;

    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM sites WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        bail!("site `{site_id}` not found");
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO site_apps (site_id, app_type, app_port, app_command, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(site_id) DO UPDATE SET
             app_type = excluded.app_type,
             app_port = excluded.app_port,
             app_command = excluded.app_command,
             updated_at = excluded.updated_at",
    )
    .bind(site_id)
    .bind(kind.as_str())
    .bind(app_port)
    .bind(app_command.trim())
    .bind(&now)
    .execute(pool)
    .await?;

    get_app(pool, site_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("app config vanished after insert"))
}

/// Remove a site's app configuration from the database.
pub async fn delete_app(pool: &SqlitePool, site_id: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM site_apps WHERE site_id = ?")
        .bind(site_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Store the config and (re)start the supervised process.
pub async fn configure(
    pool: &SqlitePool,
    paths: &Paths,
    site_id: &str,
    app_type: &str,
    app_port: i64,
    app_command: &str,
) -> Result<SiteApp> {
    let site = load_site(pool, site_id).await?;
    let app = configure_app(pool, site_id, app_type, app_port, app_command).await?;
    restart_app(&site, &app, paths).await?;
    Ok(app)
}

/// Report the stored config and the live state of a site's app process.
pub async fn status(pool: &SqlitePool, paths: &Paths, site_id: &str) -> Result<AppStatus> {
    let site = load_site(pool, site_id).await?;
    let app = get_app(pool, site_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no app is configured for site `{site_id}`"))?;
    app_status(&site, &app, paths).await
}

/// Stop a site's app process, leaving its configuration intact.
pub async fn stop(pool: &SqlitePool, paths: &Paths, site_id: &str) -> Result<()> {
    let site = load_site(pool, site_id).await?;
    let app = get_app(pool, site_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no app is configured for site `{site_id}`"))?;
    stop_app(&site, &app, paths).await
}

async fn load_site(pool: &SqlitePool, site_id: &str) -> Result<SiteRecord> {
    sqlx::query_as::<_, SiteRecord>("SELECT * FROM sites WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("site `{site_id}` not found"))
}

/// The unprivileged account a site's application runs as.
fn run_user(site: &SiteRecord) -> Result<String> {
    let user = site.pool_user.trim();
    if user.is_empty() {
        bail!("site {} has no pool user; refusing to run its app", site.id);
    }
    if user == "root" || user == "0" {
        bail!("a site app must never run as root");
    }
    if !user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("invalid site pool user `{user}`");
    }
    Ok(user.to_string())
}

/// Filesystem-safe component derived from a site id.
pub fn safe_component(site_id: &str) -> String {
    let cleaned: String = site_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-');
    let truncated: String = cleaned.chars().take(24).collect();
    let truncated = truncated.trim_matches('-');
    if truncated.is_empty() {
        "site".to_string()
    } else {
        truncated.to_string()
    }
}

/// The systemd unit name for a site.
pub fn unit_name(site_id: &str) -> String {
    format!("synthpanel-site-{}.service", safe_component(site_id))
}

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

/// Binary overrides (used by command construction and by the supervisor).
pub fn systemctl_bin() -> String {
    std::env::var("SYNTHPANEL_SYSTEMCTL_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "systemctl".to_string())
}

pub fn nohup_bin() -> String {
    std::env::var("SYNTHPANEL_NOHUP_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "nohup".to_string())
}

pub fn sh_bin() -> String {
    std::env::var("SYNTHPANEL_SH_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

fn unit_dir() -> PathBuf {
    env_path("SYNTHPANEL_SYSTEMD_UNIT_DIR", "/etc/systemd/system")
}

fn app_dir(paths: &Paths) -> PathBuf {
    paths.run_dir.join("site-apps")
}

fn script_path(paths: &Paths, site_id: &str) -> PathBuf {
    app_dir(paths).join(format!("{}.sh", safe_component(site_id)))
}

fn pid_path(paths: &Paths, site_id: &str) -> PathBuf {
    app_dir(paths).join(format!("{}.pid", safe_component(site_id)))
}

fn log_path(paths: &Paths, site_id: &str) -> PathBuf {
    app_dir(paths).join(format!("{}.log", safe_component(site_id)))
}

fn unit_path(site_id: &str) -> PathBuf {
    unit_dir().join(unit_name(site_id))
}

/// Quote a value for a POSIX shell (`/bin/sh -c`).
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote a value for a systemd unit assignment.
fn unit_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Systemd accepts bare paths, but a path with whitespace must be quoted.
fn unit_path_value(value: &str) -> String {
    if value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        unit_quote(value)
    } else {
        value.to_string()
    }
}

/// Render the wrapper script that starts the app in its document root.
///
/// Putting the command in a script lets `app_command` contain environment
/// assignments (`PORT=3000 npm start`) without systemd's ExecStart parsing
/// getting in the way, and gives the nohup fallback a stable PID to track.
pub fn render_wrapper_script(app: &SiteApp, document_root: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # Managed by SynthPanel - do not edit manually\n\
         cd {} || exit 1\n\
         export PORT={}\n\
         exec {}\n",
        shell_quote(document_root),
        app.app_port,
        app.app_command.trim()
    )
}

/// Render a per-site systemd service unit.
///
/// `User=` is always set to the site's pool user and is validated before this
/// function is called by the supervisor; the renderer also defends against a
/// caller passing root by accident.
pub fn render_unit(
    site_id: &str,
    app: &SiteApp,
    user: &str,
    document_root: &str,
    paths: &Paths,
) -> String {
    let kind = app.kind().unwrap_or(AppType::Node);
    let env = match kind {
        AppType::Node => "Environment=NODE_ENV=production",
        AppType::Python => "Environment=PYTHONUNBUFFERED=1",
    };
    let log = log_path(paths, site_id);
    format!(
        "[Unit]\n\
         Description=SynthPanel {kind} app for site {site_id}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={user}\n\
         WorkingDirectory={root}\n\
         Environment=PORT={port}\n\
         {env}\n\
         ExecStart={script}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         TimeoutStopSec=20\n\
         KillMode=mixed\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         UMask=0027\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        root = unit_quote(document_root),
        port = app.app_port,
        script = unit_path_value(&script_path(paths, site_id).display().to_string()),
        log = log.display(),
    )
}

/// A command to be executed, with an optional unprivileged account to drop to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub run_as: Option<String>,
}

/// What to ask systemctl to do with a site's unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Restart,
    Stop,
    IsActive,
    MainPid,
    DaemonReload,
}

/// Build a `systemctl` command. Kept pure so it can be unit-tested.
pub fn systemctl_command(action: ServiceAction, unit: &str, bin: &str) -> CommandSpec {
    let args = match action {
        ServiceAction::Start => vec!["--no-pager", "enable", "--now", unit],
        ServiceAction::Restart => vec!["--no-pager", "restart", unit],
        ServiceAction::Stop => vec!["--no-pager", "disable", "--now", unit],
        ServiceAction::IsActive => vec!["--no-pager", "is-active", unit],
        ServiceAction::MainPid => {
            vec!["--no-pager", "show", "-p", "MainPID", "--value", unit]
        }
        ServiceAction::DaemonReload => vec!["--no-pager", "daemon-reload"],
    };
    CommandSpec {
        program: bin.to_string(),
        args: args.into_iter().map(String::from).collect(),
        run_as: None,
    }
}

/// Build the detached fallback command. The child drops to the site user
/// before executing `nohup`.
pub fn nohup_command(user: &str, script: &Path, nohup: &str, shell: &str) -> CommandSpec {
    CommandSpec {
        program: nohup.to_string(),
        args: vec![shell.to_string(), script.display().to_string()],
        run_as: Some(user.to_string()),
    }
}

/// Whether a usable systemd instance is present.
pub async fn systemctl_available() -> bool {
    let output = Command::new(systemctl_bin())
        .args(["--no-pager", "show", "-p", "Version", "--value"])
        .output()
        .await;
    match output {
        Ok(output) => {
            output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        }
        Err(_) => false,
    }
}

static PASSWD_LOCK: Mutex<()> = Mutex::new(());

/// Resolve a user's uid/gid without ever returning uid 0.
fn resolve_user(user: &str) -> Result<(u32, u32)> {
    run_user_check(user)?;
    let name = CString::new(user).map_err(|_| anyhow::anyhow!("invalid user name `{user}`"))?;
    let _guard = PASSWD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // SAFETY: `getpwnam` returns either null or a statically owned passwd
    // entry; the mutex serialises the non-reentrant call across threads.
    unsafe {
        let entry = libc::getpwnam(name.as_ptr());
        if entry.is_null() {
            bail!("site user `{user}` does not exist on this host");
        }
        let uid = (*entry).pw_uid;
        let gid = (*entry).pw_gid;
        if uid == 0 {
            bail!("site app must not run as root");
        }
        Ok((uid, gid))
    }
}

fn run_user_check(user: &str) -> Result<()> {
    if user.is_empty() || user == "root" || user == "0" {
        bail!("site app must not run as root");
    }
    Ok(())
}

fn command_from_spec(spec: &CommandSpec) -> Result<Command> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(user) = &spec.run_as {
        let (uid, gid) = resolve_user(user)?;
        use std::os::unix::process::CommandExt;
        // `resolve_user` rejects uid 0; the child is never root.
        command.as_std_mut().gid(gid);
        command.as_std_mut().uid(uid);
    }
    Ok(command)
}

async fn run_checked(spec: &CommandSpec) -> Result<()> {
    let output = command_from_spec(spec)?.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "`{} {}` failed: {}",
            spec.program,
            spec.args.join(" "),
            stderr
        );
    }
    Ok(())
}

fn write_files(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<PathBuf> {
    let user = run_user(site)?;
    fs::create_dir_all(app_dir(paths))?;
    let script = script_path(paths, &site.id);
    fs::write(&script, render_wrapper_script(app, &site.document_root))
        .with_context(|| format!("cannot write {}", script.display()))?;
    fs::set_permissions(&script, Permissions::from_mode(0o755))?;
    let unit = render_unit(&site.id, app, &user, &site.document_root, paths);
    fs::write(unit_path(&site.id), unit).with_context(|| {
        format!(
            "cannot write systemd unit {} (set SYNTHPANEL_SYSTEMD_UNIT_DIR for tests)",
            unit_path(&site.id).display()
        )
    })?;
    Ok(script)
}

/// Start or restart a site's application process.
pub async fn start_app(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<()> {
    let user = run_user(site)?;
    if systemctl_available().await {
        write_files(site, app, paths)?;
        let unit = unit_name(&site.id);
        let bin = systemctl_bin();
        run_checked(&systemctl_command(ServiceAction::DaemonReload, &unit, &bin)).await?;
        run_checked(&systemctl_command(ServiceAction::Start, &unit, &bin)).await?;
        let _ = fs::remove_file(pid_path(paths, &site.id));
    } else {
        start_fallback(site, app, paths, &user).await?;
    }
    Ok(())
}

/// Restart a site's application process (starts it if it is not running).
pub async fn restart_app(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<()> {
    if systemctl_available().await {
        write_files(site, app, paths)?;
        let unit = unit_name(&site.id);
        let bin = systemctl_bin();
        run_checked(&systemctl_command(ServiceAction::DaemonReload, &unit, &bin)).await?;
        run_checked(&systemctl_command(ServiceAction::Restart, &unit, &bin)).await?;
        let _ = fs::remove_file(pid_path(paths, &site.id));
    } else {
        stop_fallback(site, paths, &user_for(site)?).await;
        start_fallback(site, app, paths, &user_for(site)?).await?;
    }
    Ok(())
}

fn user_for(site: &SiteRecord) -> Result<String> {
    run_user(site)
}

/// Stop a site's application process.
pub async fn stop_app(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<()> {
    let user = run_user(site)?;
    if systemctl_available().await {
        let unit = unit_name(&site.id);
        let bin = systemctl_bin();
        run_checked(&systemctl_command(ServiceAction::Stop, &unit, &bin)).await?;
    } else {
        stop_fallback(site, paths, &user).await;
    }
    let _ = app;
    Ok(())
}

async fn start_fallback(site: &SiteRecord, app: &SiteApp, paths: &Paths, user: &str) -> Result<()> {
    let script = write_files_fallback(site, app, paths)?;
    let log = File::create(log_path(paths, &site.id))
        .with_context(|| format!("cannot open {}", log_path(paths, &site.id).display()))?;
    let spec = nohup_command(user, &script, &nohup_bin(), &sh_bin());
    let mut command = command_from_spec(&spec)?;
    let child = command
        .current_dir(&site.document_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .kill_on_drop(false)
        .spawn()
        .with_context(|| format!("cannot start app for site {}", site.id))?;
    let pid = child.id().context("spawned app has no pid")?;
    fs::write(pid_path(paths, &site.id), pid.to_string())?;
    Ok(())
}

fn write_files_fallback(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<PathBuf> {
    fs::create_dir_all(app_dir(paths))?;
    let script = script_path(paths, &site.id);
    fs::write(&script, render_wrapper_script(app, &site.document_root))
        .with_context(|| format!("cannot write {}", script.display()))?;
    fs::set_permissions(&script, Permissions::from_mode(0o755))?;
    Ok(script)
}

/// Read a PID file written by the fallback supervisor.
pub fn read_pid(paths: &Paths, site_id: &str) -> Option<i32> {
    let text = fs::read_to_string(pid_path(paths, site_id)).ok()?;
    let pid = text.trim().parse::<i32>().ok()?;
    (pid > 1).then_some(pid)
}

/// True if a process with this PID exists (signal 0, no signal sent).
pub fn process_alive(pid: i32) -> bool {
    pid > 1 && unsafe { libc::kill(pid, 0) == 0 }
}

/// Only signal a PID if it is still the one we started for this site.
fn pid_belongs_to_app(pid: i32, site: &SiteRecord, user: &str) -> bool {
    let script = safe_component(&site.id);
    let (uid, _) = match resolve_user(user) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let cmdline = match fs::read_to_string(format!("/proc/{pid}/cmdline")) {
        Ok(value) => value,
        Err(_) => return false,
    };
    if !cmdline.contains(&script) {
        return false;
    }
    let status = match fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(value) => value,
        Err(_) => return false,
    };
    status.lines().find_map(|line| {
        line.strip_prefix("Uid:")?
            .split_whitespace()
            .next()?
            .parse::<u32>()
            .ok()
    }) == Some(uid)
}

async fn stop_fallback(site: &SiteRecord, paths: &Paths, user: &str) {
    let Some(pid) = read_pid(paths, &site.id) else {
        return;
    };
    if pid_belongs_to_app(pid, site, user) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        if process_alive(pid) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    let _ = fs::remove_file(pid_path(paths, &site.id));
}

/// Live status for an already-loaded app configuration.
pub async fn app_status(site: &SiteRecord, app: &SiteApp, paths: &Paths) -> Result<AppStatus> {
    let (running, pid) = if systemctl_available().await {
        let unit = unit_name(&site.id);
        let bin = systemctl_bin();
        let active = command_from_spec(&systemctl_command(ServiceAction::IsActive, &unit, &bin))?
            .output()
            .await?;
        let running =
            active.status.success() && String::from_utf8_lossy(&active.stdout).trim() == "active";
        let pid = if running {
            let shown = command_from_spec(&systemctl_command(ServiceAction::MainPid, &unit, &bin))?
                .output()
                .await?;
            String::from_utf8_lossy(&shown.stdout)
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|pid| *pid > 0)
        } else {
            None
        };
        (running, pid)
    } else {
        let pid = read_pid(paths, &site.id);
        (
            pid.is_some_and(process_alive),
            pid.filter(|pid| process_alive(*pid)),
        )
    };
    Ok(AppStatus {
        app_type: app.app_type.clone(),
        app_port: app.app_port,
        running,
        pid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};
    use uuid::Uuid;

    fn test_paths() -> Paths {
        let base = std::env::temp_dir().join(format!("sp-app-{}", Uuid::new_v4()));
        Paths {
            etc_dir: base.join("etc"),
            lib_dir: base.join("lib"),
            log_dir: base.join("log"),
            run_dir: base.join("run"),
            www_dir: base.join("www"),
            backup_dir: base.join("backups"),
            db_path: base.join("db.sqlite"),
            socket_path: base.join("run/core.sock"),
            ipc_token_path: base.join("run/ipc-token"),
            tls_dir: base.join("etc/tls"),
            secret_key_path: base.join("lib/secret.key"),
        }
    }

    fn test_app() -> SiteApp {
        SiteApp {
            site_id: "site-1234".into(),
            app_type: "node".into(),
            app_port: 3000,
            app_command: "PORT=3000 npm start --production".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn test_site() -> SiteRecord {
        SiteRecord {
            id: "site-1234".into(),
            domain: "app.example.com".into(),
            aliases: "[]".into(),
            document_root: "/var/www/sp_site/public".into(),
            template: "node".into(),
            php_version: None,
            pool_user: "sp_site".into(),
            status: "active".into(),
            ssl_mode: "none".into(),
            ssl_cert_id: None,
            owner_id: None,
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    #[test]
    fn app_type_and_port_validation() {
        assert_eq!("node".parse::<AppType>().unwrap(), AppType::Node);
        assert_eq!("Python3".parse::<AppType>().unwrap(), AppType::Python);
        assert!("ruby".parse::<AppType>().is_err());
        assert_eq!(validate_app_port(1).unwrap(), 1);
        assert_eq!(validate_app_port(65535).unwrap(), 65535);
        assert!(validate_app_port(0).is_err());
        assert!(validate_app_port(65536).is_err());
        assert!(validate_app_command("npm start").is_ok());
        assert!(validate_app_command("npm start\nrm -rf /").is_err());
        assert!(validate_app_command("sudo npm start").is_err());
    }

    #[test]
    fn shell_and_unit_quoting() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(unit_quote("/var/www/my site"), "\"/var/www/my site\"");
    }

    #[test]
    fn unit_names_are_safe() {
        assert_eq!(
            unit_name("550e8400-e29b-41d4-a716-446655440000"),
            "synthpanel-site-550e8400-e29b-41d4-a716.service"
        );
        assert_eq!(
            unit_name("../../etc/passwd"),
            "synthpanel-site-etc-passwd.service"
        );
    }

    #[test]
    fn wrapper_script_runs_as_a_single_command_in_the_document_root() {
        let script = render_wrapper_script(&test_app(), "/var/www/sp_site/public");
        assert!(script.starts_with("#!/bin/sh"));
        assert!(script.contains("cd '/var/www/sp_site/public' || exit 1"));
        assert!(script.contains("export PORT=3000"));
        assert!(script.contains("exec PORT=3000 npm start --production"));
        assert!(script.lines().count() <= 5);
    }

    #[test]
    fn unit_runs_as_site_user_and_never_as_root() {
        let paths = test_paths();
        let unit = render_unit(
            "site-1234",
            &test_app(),
            "sp_site",
            "/var/www/sp_site/public",
            &paths,
        );
        assert!(unit.contains("User=sp_site"));
        assert!(!unit.contains("User=root"));
        assert!(unit.contains("WorkingDirectory=\"/var/www/sp_site/public\""));
        assert!(unit.contains("Environment=PORT=3000"));
        assert!(unit.contains("Environment=NODE_ENV=production"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains(&format!(
            "ExecStart={}",
            script_path(&paths, "site-1234").display()
        )));
        assert!(unit.contains("StandardOutput=append:"));
    }

    #[test]
    fn python_unit_sets_python_environment() {
        let mut app = test_app();
        app.app_type = "python".into();
        app.app_command = "python3 app.py".into();
        let unit = render_unit("site-1234", &app, "sp_site", "/srv/app", &test_paths());
        assert!(unit.contains("Description=SynthPanel python app"));
        assert!(unit.contains("Environment=PYTHONUNBUFFERED=1"));
        assert!(!unit.contains("NODE_ENV"));
    }

    #[test]
    fn command_specs_are_built_without_shell_interpolation() {
        let unit = "synthpanel-site-abc.service";
        assert_eq!(
            systemctl_command(ServiceAction::Start, unit, "systemctl"),
            CommandSpec {
                program: "systemctl".into(),
                args: vec!["--no-pager", "enable", "--now", unit]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                run_as: None,
            }
        );
        assert_eq!(
            systemctl_command(ServiceAction::Restart, unit, "systemctl").args,
            vec!["--no-pager", "restart", unit]
        );
        assert_eq!(
            systemctl_command(ServiceAction::Stop, unit, "systemctl").args,
            vec!["--no-pager", "disable", "--now", unit]
        );
        assert_eq!(
            systemctl_command(ServiceAction::IsActive, unit, "systemctl").args,
            vec!["--no-pager", "is-active", unit]
        );
        assert_eq!(
            systemctl_command(ServiceAction::MainPid, unit, "systemctl").args,
            vec!["--no-pager", "show", "-p", "MainPID", "--value", unit]
        );
        let script = Path::new("/run/synthpanel/site-apps/abc.sh");
        assert_eq!(
            nohup_command("sp_site", script, "nohup", "/bin/sh"),
            CommandSpec {
                program: "nohup".into(),
                args: vec!["/bin/sh".into(), script.display().to_string()],
                run_as: Some("sp_site".into()),
            }
        );
    }

    #[test]
    fn root_is_never_an_allowed_run_user() {
        let mut site = test_site();
        site.pool_user = "root".into();
        assert!(run_user(&site).is_err());
        site.pool_user = String::new();
        assert!(run_user(&site).is_err());
        assert!(run_user_check("0").is_err());
        assert_eq!(run_user(&test_site()).unwrap(), "sp_site");
    }

    #[tokio::test]
    async fn app_config_roundtrips_and_is_scoped_to_a_site() {
        let path = std::env::temp_dir().join(format!("sp-app-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let sites = Sites::new(pool.clone());
        let site = sites
            .create(CreateSite {
                domain: "app.example.com".into(),
                aliases: vec![],
                document_root: "/var/www/sp_app/public".into(),
                template: "node".into(),
                php_version: None,
                ssl_mode: "none".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap();
        assert!(get_app(&pool, &site.id).await.unwrap().is_none());

        let saved = configure_app(&pool, &site.id, "node", 3100, "npm start")
            .await
            .unwrap();
        assert_eq!(saved.app_type, "node");
        assert_eq!(saved.app_port, 3100);
        assert_eq!(saved.kind().unwrap(), AppType::Node);

        let replaced = configure_app(&pool, &site.id, "python", 4200, "python3 app.py")
            .await
            .unwrap();
        assert_eq!(replaced.app_type, "python");
        assert_eq!(replaced.app_port, 4200);
        assert!(get_app(&pool, "missing-site").await.unwrap().is_none());
        assert!(
            configure_app(&pool, "missing-site", "node", 3000, "npm start")
                .await
                .is_err()
        );
        assert!(delete_app(&pool, &site.id).await.unwrap());
        assert!(get_app(&pool, &site.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn status_serializes_to_the_ipc_contract() {
        let status = AppStatus {
            app_type: "node".into(),
            app_port: 3000,
            running: true,
            pid: Some(4242),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["app_type"], "node");
        assert_eq!(json["app_port"], 3000);
        assert_eq!(json["running"], true);
        assert_eq!(json["pid"], 4242);

        let status = AppStatus {
            app_type: "python".into(),
            app_port: 8000,
            running: false,
            pid: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("pid").is_none());
        assert_eq!(json["running"], false);
    }
}
