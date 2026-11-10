//! Administrative endpoints: settings and user management.

use crate::api::auth::{audit, sanitize_user};
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use std::sync::Arc;

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Map, Value, json};
use synthpanel_ipc::methods;

/// Settings that may be changed through the panel.
const ALLOWED_SETTINGS: &[&str] = &[
    "hostname",
    "timezone",
    "locale",
    "panel_port",
    "session_lifetime",
    "update_channel",
    "maintenance_mode",
];

pub async fn settings_get(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let settings: Map<String, Value> = state.core_call(methods::SETTINGS_LIST, json!({})).await?;
    Ok(Json(Value::Object(settings)))
}

pub async fn settings_patch(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<Map<String, Value>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    if body.is_empty() {
        return Err(ApiError::bad_request("no settings supplied"));
    }
    for (key, value) in &body {
        if !ALLOWED_SETTINGS.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!("unknown setting `{key}`")));
        }
        let _: Value = state
            .core_call(methods::SETTINGS_SET, json!({ "key": key, "value": value }))
            .await?;
    }
    audit(
        &state,
        &user.user_id,
        "",
        "settings.update",
        &body.keys().cloned().collect::<Vec<_>>().join(","),
        &user.ip,
    )
    .await;
    let settings: Map<String, Value> = state.core_call(methods::SETTINGS_LIST, json!({})).await?;
    Ok(Json(Value::Object(settings)))
}

pub async fn users_list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let users: Vec<Value> = state.core_call(methods::USERS_LIST, json!({})).await?;
    let sanitized: Vec<Value> = users.iter().map(sanitize_user).collect();
    Ok(Json(json!({ "users": sanitized })))
}

pub async fn users_create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;
    let email = body
        .get("email")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("email is required"))?
        .trim()
        .to_lowercase();
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .unwrap_or_else(|| email.clone());
    let password = body
        .get("password")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("password is required"))?;
    if password.len() < 12 {
        return Err(ApiError::bad_request(
            "the password must be at least 12 characters long",
        ));
    }
    let role = body
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("operator")
        .to_string();
    if !matches!(role.as_str(), "admin" | "operator" | "user") {
        return Err(ApiError::bad_request(format!("unknown role `{role}`")));
    }
    let quota_sites = body
        .get("quota_sites")
        .and_then(Value::as_i64)
        .unwrap_or(10);
    let quota_dbs = body.get("quota_dbs").and_then(Value::as_i64).unwrap_or(10);

    let password_hash = crate::auth::hash_password(password)
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let created: Value = state
        .core_call(
            methods::USERS_CREATE,
            json!({
                "email": email,
                "name": name,
                "password_hash": password_hash,
                "role": role,
                "quota_sites": quota_sites,
                "quota_dbs": quota_dbs,
            }),
        )
        .await?;
    audit(&state, &user.user_id, "", "users.create", &email, &user.ip).await;
    Ok(Json(sanitize_user(&created)))
}

pub async fn users_update(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &user).await?;

    let mut patch = Map::new();
    for key in ["email", "name", "role", "status"] {
        if let Some(value) = body.get(key).and_then(Value::as_str) {
            patch.insert(key.to_string(), json!(value));
        }
    }
    for key in ["quota_sites", "quota_dbs"] {
        if let Some(value) = body.get(key).and_then(Value::as_i64) {
            patch.insert(key.to_string(), json!(value));
        }
    }
    if let Some(password) = body.get("password").and_then(Value::as_str) {
        if password.len() < 12 {
            return Err(ApiError::bad_request(
                "the password must be at least 12 characters long",
            ));
        }
        let hash = crate::auth::hash_password(password)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        patch.insert("password_hash".to_string(), json!(hash));
    }
    if patch.is_empty() {
        return Err(ApiError::bad_request("nothing to update"));
    }
    patch.insert("id".to_string(), json!(id));

    let updated: Option<Value> = state
        .core_call(methods::USERS_UPDATE, Value::Object(patch))
        .await?;
    let Some(updated) = updated else {
        return Err(ApiError::not_found("user not found"));
    };
    audit(&state, &user.user_id, "", "users.update", &id, &user.ip).await;
    Ok(Json(sanitize_user(&updated)))
}

/// Ensure the authenticated user has the admin role.
pub async fn require_admin(state: &AppState, user: &AuthUser) -> Result<Value, ApiError> {
    let record: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": user.user_id }))
        .await?;
    let record = record.ok_or_else(|| ApiError::unauthorized("user no longer exists"))?;
    let role = record.get("role").and_then(Value::as_str).unwrap_or("");
    if role != "admin" {
        return Err(ApiError::forbidden("administrator role required"));
    }
    Ok(record)
}
