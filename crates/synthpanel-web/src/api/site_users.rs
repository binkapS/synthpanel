//! Per-site SSH user endpoints.

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
pub struct AddSiteUserRequest {
    pub username: String,
    pub public_key: String,
}

/// `GET /api/v1/sites/{site_id}/ssh-users`
pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(site_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let users: Vec<Value> = state
        .core_call(methods::SITE_SSH_USERS_LIST, json!({ "site_id": site_id }))
        .await?;
    Ok(Json(json!({ "users": users })))
}

/// `POST /api/v1/sites/{site_id}/ssh-users`
pub async fn add(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(site_id): Path<String>,
    ApiJson(body): ApiJson<AddSiteUserRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SITE_SSH_USER_ADD,
            json!({
                "site_id": site_id,
                "username": body.username,
                "public_key": body.public_key,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "site_ssh_users.add",
        &format!("{site_id}/{}", body.username),
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// `DELETE /api/v1/sites/{site_id}/ssh-users/{username}`
pub async fn remove(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path((site_id, username)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SITE_SSH_USER_REMOVE,
            json!({ "site_id": site_id, "username": username }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "site_ssh_users.remove",
        &format!("{site_id}/{username}"),
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
