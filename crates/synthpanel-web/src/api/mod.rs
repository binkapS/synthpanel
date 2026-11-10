//! REST API v1.

pub mod admin;
pub mod auth;
pub mod dashboard;
pub mod sites;

use crate::state::AppState;
use axum::Router;
use axum::routing::{get, patch, post};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/auth/2fa/setup", post(auth::totp_setup))
        .route("/api/v1/auth/2fa/enable", post(auth::totp_enable))
        .route("/api/v1/auth/2fa/verify", post(auth::totp_verify))
        .route("/api/v1/auth/2fa/disable", post(auth::totp_disable))
        .route("/api/v1/metrics/snapshot", get(dashboard::metrics_snapshot))
        .route("/api/v1/metrics/range", get(dashboard::metrics_range))
        .route("/api/v1/services", get(dashboard::services))
        .route("/api/v1/audit", get(dashboard::audit))
        .route(
            "/api/v1/settings",
            get(admin::settings_get).patch(admin::settings_patch),
        )
        .route(
            "/api/v1/users",
            get(admin::users_list).post(admin::users_create),
        )
        .route("/api/v1/users/{id}", patch(admin::users_update))
        .route("/api/v1/sites", get(sites::list).post(sites::create))
        .route(
            "/api/v1/sites/{id}",
            get(sites::get).patch(sites::update).delete(sites::delete),
        )
        .with_state(state)
}
