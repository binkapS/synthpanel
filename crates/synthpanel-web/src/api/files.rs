//! File manager endpoints.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize, Default)]
pub struct PathQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MkdirRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChmodRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ArchiveRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub path: String,
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::FILES_LIST,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": query.path.unwrap_or_default(),
            }),
        )
        .await?;
    Ok(Json(result))
}

pub async fn read(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query
        .path
        .ok_or_else(|| ApiError::bad_request("a path is required"))?;
    let result: Value = state
        .core_call(
            methods::FILES_READ,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": path,
            }),
        )
        .await?;
    Ok(Json(result))
}

pub async fn write(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<WriteRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "scope": body.scope.unwrap_or_else(|| "www".into()),
        "path": body.path,
    });
    if let Some(content) = &body.content {
        params["content"] = json!(content);
    }
    if let Some(encoded) = &body.content_base64 {
        params["content_base64"] = json!(encoded);
    }
    let result: Value = state.core_call(methods::FILES_WRITE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.write",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn mkdir(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<MkdirRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::FILES_MKDIR,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.mkdir",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<RenameRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::FILES_RENAME,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "from": body.from,
                "to": body.to,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.rename",
        &body.from,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query
        .path
        .ok_or_else(|| ApiError::bad_request("a path is required"))?;
    let result: Value = state
        .core_call(
            methods::FILES_DELETE,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": path,
            }),
        )
        .await?;
    audit(&state, &user.user_id, "", "files.delete", &path, &user.ip).await;
    Ok(Json(result))
}

pub async fn chmod(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ChmodRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "scope": body.scope.unwrap_or_else(|| "www".into()),
        "path": body.path,
    });
    if let Some(mode) = &body.mode {
        params["mode"] = json!(mode);
    }
    if let Some(owner) = &body.owner {
        params["owner"] = json!(owner);
    }
    if let Some(group) = &body.group {
        params["group"] = json!(group);
    }
    let result: Value = state.core_call(methods::FILES_CHMOD, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.chmod",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn archive(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ArchiveRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::FILES_ARCHIVE,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
                "format": body.format.unwrap_or_else(|| "tar.gz".into()),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.archive",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn extract(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ExtractRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::FILES_EXTRACT,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
                "into": body.into.unwrap_or_default(),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.extract",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
