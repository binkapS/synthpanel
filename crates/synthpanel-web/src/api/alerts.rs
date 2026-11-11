//! Alerting endpoints: channels, rules and the alert history.

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
pub struct ChannelRequest {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Deserialize, Default)]
pub struct RuleRequest {
    pub name: String,
    pub event: String,
    #[serde(default)]
    pub severity: Option<String>,
    pub channel_id: String,
    #[serde(default)]
    pub cooldown_secs: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct TestRequest {
    pub channel_id: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// List the alert history.
pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let alerts: Vec<Value> = state.core_call(methods::ALERTS_LIST, json!({})).await?;
    Ok(Json(json!({ "alerts": alerts })))
}

/// List the channels.
pub async fn channels(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let channels: Vec<Value> = state
        .core_call(methods::ALERT_CHANNELS_LIST, json!({}))
        .await?;
    Ok(Json(json!({ "channels": channels })))
}

/// Create a channel.
pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ChannelRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let channel: Value = state
        .core_call(
            methods::ALERT_CHANNELS_CREATE,
            json!({
                "name": body.name,
                "kind": body.kind,
                "config": body.config,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "alert.channel.create",
        &body.name,
        &user.ip,
    )
    .await;
    Ok(Json(channel))
}

/// Delete a channel.
pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(methods::ALERT_CHANNELS_DELETE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "alert.channel.delete",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// List the rules.
pub async fn rules(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let rules: Vec<Value> = state
        .core_call(methods::ALERT_RULES_LIST, json!({}))
        .await?;
    Ok(Json(json!({ "rules": rules })))
}

/// Create a rule.
pub async fn create_rule(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<RuleRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let rule: Value = state
        .core_call(
            methods::ALERT_RULES_CREATE,
            json!({
                "name": body.name,
                "event": body.event,
                "severity": body.severity.unwrap_or_else(|| "warning".into()),
                "channel_id": body.channel_id,
                "cooldown_secs": body.cooldown_secs.unwrap_or(900),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "alert.rule.create",
        &body.name,
        &user.ip,
    )
    .await;
    Ok(Json(rule))
}

/// Delete a rule.
pub async fn delete_rule(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let result: Value = state
        .core_call(methods::ALERT_RULES_DELETE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "alert.rule.delete",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// Send a test alert to a channel.
pub async fn test(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<TestRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let message = body
        .message
        .unwrap_or_else(|| "This is a test alert from SynthPanel.".to_string());
    let result: Value = state
        .core_call(
            methods::ALERTS_TEST,
            json!({ "channel_id": body.channel_id, "message": message }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "alert.test",
        &body.channel_id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
