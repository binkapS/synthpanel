//! Framework template catalogue and one-click install endpoints.

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
pub struct InstallRequest {
    pub template: String,
    #[serde(default)]
    pub version: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Value = state.core_call(methods::TEMPLATES_LIST, json!({})).await?;
    Ok(Json(rows))
}

pub async fn install(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<InstallRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::TEMPLATES_INSTALL,
            json!({
                "site_id": id,
                "template": body.template,
                "version": body.version,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "templates.install",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
