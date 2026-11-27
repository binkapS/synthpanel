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

/// Report which panel-level gates are active.
///
/// This is deliberately configuration-only: it never returns the Basic Auth
/// username, password hash or the individual allow-list entries.
pub async fn access(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let maintenance: Value = state
        .core_call(methods::SETTINGS_GET, json!({ "key": "maintenance_mode" }))
        .await?;
    Ok(Json(json!({
        "panel_basic_auth": state.config.panel_basic_auth.is_some(),
        "ip_allow_list": state.config.ip_allow_list_enabled(),
        "ip_allow_list_entries": state.config.panel_allow_ips.len(),
        "trusted_proxies": state.config.trusted_proxies,
        "maintenance_mode": crate::middleware::setting_flag(Some(&maintenance)),
    })))
}
