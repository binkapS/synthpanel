//! Endpoints for the certificate the panel serves on its own interface.

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
pub struct IssueRequest {
    /// `webroot` (the panel serves the challenge) or `standalone`.
    #[serde(default)]
    pub method: Option<String>,
    /// The e-mail address for the ACME account.
    #[serde(default)]
    pub email: Option<String>,
}

/// Describe the certificate the panel currently serves.
pub async fn status(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state
        .core_call(methods::PANEL_SSL_STATUS, json!({}))
        .await?;
    Ok(Json(status))
}

/// Request a certificate for the panel hostname.
pub async fn issue(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<IssueRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::PANEL_SSL_ISSUE,
            json!({
                "method": body.method.unwrap_or_else(|| "webroot".into()),
                "email": body.email,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "panel.ssl.issue",
        result.get("domain").and_then(Value::as_str).unwrap_or(""),
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
