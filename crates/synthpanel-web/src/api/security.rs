//! Security endpoints: firewall, fail2ban and SSH keys.

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
pub struct FirewallRuleRequest {
    pub port: String,
    #[serde(default = "default_proto")]
    pub proto: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default = "default_action")]
    pub action: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_proto() -> String {
    "tcp".into()
}

fn default_source() -> String {
    "any".into()
}

fn default_action() -> String {
    "allow".into()
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct UnbanRequest {
    pub jail: String,
    pub ip: String,
}

#[derive(Debug, Deserialize)]
pub struct SshKeyRequest {
    pub label: String,
    pub public_key: String,
    #[serde(default = "default_user")]
    pub user: String,
}

fn default_user() -> String {
    "root".into()
}

pub async fn firewall(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state
        .core_call(methods::SECURITY_FIREWALL, json!({}))
        .await?;
    Ok(Json(status))
}

pub async fn firewall_apply(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<FirewallRuleRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SECURITY_FIREWALL_APPLY,
            json!({
                "port": body.port,
                "proto": body.proto,
                "source": body.source,
                "action": body.action,
                "comment": body.comment,
                "enabled": body.enabled,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "security.firewall.apply",
        &body.port,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn fail2ban(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state
        .core_call(methods::SECURITY_FAIL2BAN, json!({}))
        .await?;
    Ok(Json(status))
}

pub async fn fail2ban_unban(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<UnbanRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SECURITY_FAIL2BAN_UNBAN,
            json!({ "jail": body.jail, "ip": body.ip }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "security.fail2ban.unban",
        &body.ip,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn ssh_keys(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let keys: Vec<Value> = state
        .core_call(methods::SECURITY_SSH_KEYS, json!({}))
        .await?;
    Ok(Json(json!({ "keys": keys })))
}

pub async fn ssh_key_add(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<SshKeyRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SECURITY_SSH_KEY_ADD,
            json!({
                "label": body.label,
                "public_key": body.public_key,
                "user": body.user,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "security.ssh_key.add",
        &body.label,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn ssh_key_remove(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::SECURITY_SSH_KEY_REMOVE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "security.ssh_key.remove",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
