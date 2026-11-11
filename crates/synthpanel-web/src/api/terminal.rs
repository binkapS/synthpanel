//! Web terminal endpoints.
//!
//! The shell itself runs inside the privileged core; the web process only
//! relays keystrokes and output. Every endpoint requires the admin role.

use crate::api::admin::require_admin;
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

#[derive(Debug, Deserialize, Default)]
pub struct OpenRequest {
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WriteRequest {
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub data_b64: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ResizeRequest {
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
}

/// List the open sessions.
pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let sessions: Vec<Value> = state.core_call(methods::TERMINAL_LIST, json!({})).await?;
    Ok(Json(json!({ "sessions": sessions })))
}

/// Open a session.
pub async fn open(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<OpenRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let session: Value = state
        .core_call(
            methods::TERMINAL_OPEN,
            json!({
                "shell": body.shell,
                "cwd": body.cwd,
                "user": body.user,
                "rows": body.rows,
                "cols": body.cols,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "terminal.open",
        body.shell.as_deref().unwrap_or("default"),
        &user.ip,
    )
    .await;
    Ok(Json(session))
}

/// Send input to a session.
pub async fn write(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<WriteRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(
            methods::TERMINAL_WRITE,
            json!({ "id": id, "data": body.data, "data_b64": body.data_b64 }),
        )
        .await?;
    Ok(Json(result))
}

/// Read the buffered output of a session.
pub async fn read(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(methods::TERMINAL_READ, json!({ "id": id }))
        .await?;
    Ok(Json(result))
}

/// Resize a session.
pub async fn resize(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResizeRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(
            methods::TERMINAL_RESIZE,
            json!({
                "id": id,
                "rows": body.rows.unwrap_or(24),
                "cols": body.cols.unwrap_or(80),
            }),
        )
        .await?;
    Ok(Json(result))
}

/// Close a session.
pub async fn close(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(methods::TERMINAL_CLOSE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "terminal.close", &id, &user.ip).await;
    Ok(Json(result))
}
