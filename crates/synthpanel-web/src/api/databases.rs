//! Database management endpoints.

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
pub struct CreateDatabaseRequest {
    pub engine: String,
    pub name: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub privileges: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddUserRequest {
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub privileges: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let databases: Vec<Value> = state.core_call(methods::DATABASES_LIST, json!({})).await?;
    let engines: Vec<Value> = state
        .core_call(methods::DATABASES_ENGINES, json!({}))
        .await?;
    Ok(Json(json!({ "databases": databases, "engines": engines })))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let record: Value = state
        .core_call(methods::DATABASES_GET, json!({ "id": id }))
        .await?;
    if record.is_null() {
        return Err(ApiError::not_found("database not found"));
    }
    Ok(Json(record))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateDatabaseRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "engine": body.engine,
        "name": body.name,
    });
    if let Some(username) = &body.username {
        params["username"] = json!(username);
    }
    if let Some(password) = &body.password {
        params["password"] = json!(password);
    }
    if let Some(privileges) = &body.privileges {
        params["privileges"] = json!(privileges);
    }
    let result: Value = state.core_call(methods::DATABASES_CREATE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "databases.create",
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
        .core_call(methods::DATABASES_DELETE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "databases.delete", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn add_user(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AddUserRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "database_id": id,
        "username": body.username,
    });
    if let Some(password) = &body.password {
        params["password"] = json!(password);
    }
    if let Some(privileges) = &body.privileges {
        params["privileges"] = json!(privileges);
    }
    let result: Value = state.core_call(methods::DATABASES_ADD_USER, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "databases.add_user",
        &body.username,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn users(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let users: Vec<Value> = state
        .core_call(methods::DATABASES_USERS, json!({ "database_id": id }))
        .await?;
    Ok(Json(json!({ "users": users })))
}

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::DATABASES_DELETE_USER, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "databases.delete_user",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
