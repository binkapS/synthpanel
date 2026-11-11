//! Self-update endpoints.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize, Default)]
pub struct UpdateRequest {
    #[serde(default)]
    pub channel: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RollbackRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub backup_dir: Option<String>,
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state.core_call(methods::UPDATE_STATUS, json!({})).await?;
    Ok(Json(result))
}

pub async fn check(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    ApiJson(body): ApiJson<UpdateRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({});
    if let Some(channel) = &body.channel {
        params["channel"] = json!(channel);
    }
    let result: Value = state.core_call(methods::UPDATE_CHECK, params).await?;
    Ok(Json(result))
}

pub async fn apply(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<UpdateRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({});
    if let Some(channel) = &body.channel {
        params["channel"] = json!(channel);
    }
    let result: Value = state.core_call(methods::UPDATE_APPLY, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "update.apply",
        result
            .get("result")
            .and_then(|value| value.get("to_version"))
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn rollback(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<RollbackRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::UPDATE_ROLLBACK,
            json!({ "id": body.id, "backup_dir": body.backup_dir }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "update.rollback",
        body.id.as_deref().unwrap_or("latest"),
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
