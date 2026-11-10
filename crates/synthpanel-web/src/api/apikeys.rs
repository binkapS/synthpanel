//! API key management endpoints.

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
pub struct CreateKeyRequest {
    pub name: String,
    #[serde(default)]
    pub scopes: Option<Value>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let keys: Vec<Value> = state
        .core_call(methods::API_KEYS_LIST, json!({ "user_id": user.user_id }))
        .await?;
    Ok(Json(json!({ "keys": keys })))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateKeyRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "user_id": user.user_id,
        "name": body.name,
    });
    if let Some(scopes) = &body.scopes {
        params["scopes"] = scopes.clone();
    }
    let result: Value = state.core_call(methods::API_KEYS_CREATE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "api_keys.create",
        &body.name,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::API_KEYS_DELETE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "api_keys.delete", &id, &user.ip).await;
    Ok(Json(result))
}
