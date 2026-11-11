//! `synthpanel-web` — unprivileged HTTP entry point.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use synthpanel_web::config::WebConfig;
use synthpanel_web::state::AppState;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    // reqwest and axum-server can pull in both rustls providers; choose one
    // explicitly so the process-level default is deterministic.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = WebConfig::from_env();
    std::fs::create_dir_all(&config.tls_dir).context("failed to create the TLS directory")?;

    let generated = synthpanel_web::tls::ensure_certificate(
        &config.tls_cert_path(),
        &config.tls_key_path(),
        &config.panel_hostname,
    )?;
    if generated {
        info!(
            path = %config.tls_cert_path().display(),
            "generated a self-signed panel certificate"
        );
    }

    let token = std::fs::read_to_string(&config.ipc_token_path).with_context(|| {
        format!(
            "cannot read the IPC token at {} — is synthpanel-core running?",
            config.ipc_token_path.display()
        )
    })?;

    let state = Arc::new(AppState::new(
        config.clone(),
        config.core_socket.clone(),
        token.trim().to_string(),
    ));

    // Periodic housekeeping: expired sessions and stale 2FA challenges.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                state.sessions.cleanup();
                state.pending_2fa.cleanup();
            }
        });
    }

    let app = synthpanel_web::build_router(state);
    let addr: SocketAddr = config
        .bind_addr
        .parse()
        .with_context(|| format!("invalid bind address `{}`", config.bind_addr))?;

    let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        config.tls_cert_path(),
        config.tls_key_path(),
    )
    .await
    .context("failed to load the panel TLS certificate")?;

    info!(%addr, "synthpanel-web listening (https)");

    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("web server terminated")?;

    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let json = std::env::var("SYNTHPANEL_LOG_JSON")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if json {
        let _ = tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }
}
