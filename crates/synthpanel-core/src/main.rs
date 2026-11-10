//! `synthpanel-core` — privileged daemon entry point.

use anyhow::{Context, Result};
use std::path::Path;
use synthpanel_core::config::Paths;
use synthpanel_core::db;
use synthpanel_core::ipc::IpcServer;
use synthpanel_core::metrics::Collector;
use synthpanel_core::settings::Settings;
use synthpanel_core::users::Users;
use tokio::net::UnixListener;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let paths = Paths::from_env();
    paths
        .ensure_dirs()
        .context("failed to create SynthPanel directories")?;
    info!(etc = %paths.etc_dir.display(), "synthpanel-core starting");

    ensure_ipc_token(&paths.ipc_token_path).await?;
    ensure_secret_key(&paths.secret_key_path).await?;

    let pool = db::connect(&paths.db_path)
        .await
        .context("failed to open state database")?;
    db::migrate(&pool)
        .await
        .context("failed to apply database migrations")?;

    let settings = Settings::new(pool.clone());
    let users = Users::new(pool.clone());

    let listener = bind_socket(&paths)?;
    info!(socket = %paths.socket_path.display(), "ipc socket listening");

    let host = sysinfo::System::host_name().unwrap_or_else(|| "localhost".to_string());
    let sites = synthpanel_core::sites::Sites::new(pool.clone());
    let databases = synthpanel_core::databases::Databases::new(pool.clone());
    let apply = synthpanel_core::apply::ApplyPipeline::new();
    let stores = synthpanel_core::ipc::Stores {
        settings,
        users,
        sites,
        databases,
    };
    let server = IpcServer::new(paths.clone(), pool.clone(), stores, apply, host.clone());

    // Metrics collector: samples every 10s, prunes raw data hourly.
    let collector = Collector::new(pool.clone());
    tokio::spawn(collector.run());
    {
        let pool = pool.clone();
        let host = host.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match synthpanel_core::metrics::prune(&pool, &host).await {
                    Ok(removed) if removed > 0 => {
                        info!(removed, "pruned old metrics samples")
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%error, "failed to prune metrics"),
                }
            }
        });
    }

    tokio::spawn(async move {
        wait_for_shutdown().await;
        info!("shutdown signal received");
        // The IPC listener is owned by `serve`; systemd escalates to SIGKILL
        // if the process does not exit in time.
        std::process::exit(0);
    });

    server
        .serve(listener)
        .await
        .context("ipc server terminated")?;
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let json = std::env::var("SYNTHPANEL_LOG_JSON")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if json {
        let _ = tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .try_init();
    }
}

async fn ensure_ipc_token(path: &Path) -> Result<()> {
    if let Ok(contents) = tokio::fs::read_to_string(path).await {
        let token = contents.trim();
        if !token.is_empty() {
            return Ok(());
        }
    }
    let token = generate_secure_token(32);
    tokio::fs::write(path, &token)
        .await
        .context("failed to write IPC token")?;
    restrict_file(path, 0o640);
    info!(path = %path.display(), "generated IPC token");
    Ok(())
}

async fn ensure_secret_key(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let key = generate_secure_token(32);
    tokio::fs::write(path, &key)
        .await
        .context("failed to write secret key")?;
    restrict_file(path, 0o600);
    info!(path = %path.display(), "generated secret key");
    Ok(())
}

fn generate_secure_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn restrict_file(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    chown_to_synthpanel(path);
}

/// Best-effort chown to root:synthpanel. Fails silently when the user or group
/// does not exist (development) or when the process is not root.
fn chown_to_synthpanel(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    unsafe {
        let user = CString::new("synthpanel").unwrap();
        let group = CString::new("synthpanel").unwrap();
        let pw = libc::getpwnam(user.as_ptr());
        let gr = libc::getgrnam(group.as_ptr());
        if pw.is_null() || gr.is_null() {
            return;
        }
        libc::chown(cpath.as_ptr(), (*pw).pw_uid, (*gr).gr_gid);
    }
}

fn bind_socket(paths: &Paths) -> Result<UnixListener> {
    let socket_path = &paths.socket_path;
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
    chown_to_synthpanel(socket_path);
    Ok(listener)
}

async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
