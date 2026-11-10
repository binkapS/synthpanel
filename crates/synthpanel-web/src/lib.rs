//! SynthPanel web process: HTTP API, session handling and the embedded SPA.

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod extract;
pub mod ipc;
pub mod middleware;
pub mod ratelimit;
pub mod session;
pub mod state;
pub mod static_files;
pub mod tls;
pub mod totp;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::sync::Arc;

use crate::state::AppState;

/// Build the complete HTTP application.
pub fn build_router(state: Arc<AppState>) -> Router {
    let api = api::router(Arc::clone(&state));

    Router::new()
        .merge(api)
        .route("/healthz", get(healthz))
        .fallback(spa_fallback)
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            middleware::csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            middleware::security_headers,
        ))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Serve the SPA for client-side routes, JSON 404 for unknown API paths.
async fn spa_fallback(uri: Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": { "code": "not_found", "message": "unknown endpoint" }
            })),
        )
            .into_response();
    }
    static_files::serve(uri.path()).await
}
