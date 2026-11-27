//! WordPress one-click install endpoint.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize)]
pub struct InstallWordPressRequest {
    pub admin_user: String,
    pub admin_password: String,
    pub admin_email: String,
}

/// `POST /api/v1/sites/{site_id}/wordpress/install`
///
/// Registers this handler in `api/mod.rs`:
/// `.route("/api/v1/sites/{site_id}/wordpress/install", post(wordpress::install))`
///
/// The site id is placed in the IPC params alongside the admin credentials;
/// the privileged core performs the filesystem and database work.
pub async fn install(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(site_id): Path<String>,
    ApiJson(body): ApiJson<InstallWordPressRequest>,
) -> Result<Json<Value>, ApiError> {
    if site_id.trim().is_empty() {
        return Err(ApiError::bad_request("site_id must not be empty"));
    }
    if body.admin_user.trim().is_empty() {
        return Err(ApiError::bad_request("admin_user must not be empty"));
    }
    if body.admin_password.is_empty() {
        return Err(ApiError::bad_request("admin_password must not be empty"));
    }
    let email = body.admin_email.trim();
    if email.len() > 254
        || email.chars().any(char::is_whitespace)
        || email.matches('@').count() != 1
        || email.starts_with('@')
        || email.ends_with('@')
    {
        return Err(ApiError::bad_request("admin_email is not valid"));
    }

    let result: Value = state
        .core_call(
            methods::WORDPRESS_INSTALL,
            json!({
                "site_id": site_id,
                "admin_user": body.admin_user,
                "admin_password": body.admin_password,
                "admin_email": email,
            }),
        )
        .await?;

    audit(
        &state,
        &user.user_id,
        "",
        "wordpress.install",
        &site_id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
