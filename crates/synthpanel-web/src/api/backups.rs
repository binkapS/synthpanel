//! Backup endpoints.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize)]
pub struct RunBackupRequest {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub contents: Option<Value>,
    #[serde(default)]
    pub destination: Option<Value>,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub schedule_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub destination: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DownloadQuery {
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScheduleRequest {
    pub name: String,
    pub cron: String,
    pub destination: Value,
    #[serde(default)]
    pub retention: Option<Value>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let backups: Vec<Value> = state.core_call(methods::BACKUPS_LIST, json!({})).await?;
    let schedules: Vec<Value> = state
        .core_call(methods::BACKUPS_SCHEDULES, json!({}))
        .await?;
    Ok(Json(json!({
        "backups": backups,
        "schedules": schedules,
    })))
}

pub async fn run(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<RunBackupRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "kind": body.kind.unwrap_or_else(|| if body.site_id.is_some() { "site".into() } else { "full".into() }),
    });
    if let Some(site_id) = &body.site_id {
        params["site_id"] = json!(site_id);
    }
    if let Some(contents) = &body.contents {
        params["contents"] = contents.clone();
    }
    if let Some(destination) = &body.destination {
        params["destination"] = destination.clone();
    }
    if let Some(passphrase) = &body.passphrase {
        params["passphrase"] = json!(passphrase);
    }
    if let Some(schedule_id) = &body.schedule_id {
        params["schedule_id"] = json!(schedule_id);
    }
    let result: Value = state.core_call(methods::BACKUPS_CREATE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "backups.create",
        body.site_id.as_deref().unwrap_or("full"),
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
        .core_call(methods::BACKUPS_DELETE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "backups.delete", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn verify(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RestoreRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::BACKUPS_VERIFY,
            json!({ "id": id, "passphrase": body.passphrase, "destination": body.destination }),
        )
        .await?;
    Ok(Json(result))
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RestoreRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::BACKUPS_RESTORE,
            json!({ "id": id, "passphrase": body.passphrase, "destination": body.destination }),
        )
        .await?;
    audit(&state, &user.user_id, "", "backups.restore", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, ApiError> {
    let info: Value = state
        .core_call(
            methods::BACKUPS_DOWNLOAD,
            json!({ "id": id, "passphrase": query.passphrase }),
        )
        .await?;
    let path = info
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::internal("the core returned no archive path"))?;
    let name = info
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("backup.tar.gz");
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| ApiError::internal(format!("cannot read the archive: {error}")))?;

    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    let disposition = format!("attachment; filename=\"{name}\"");
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

pub async fn schedule_create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ScheduleRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "name": body.name,
        "cron": body.cron,
        "destination": body.destination,
    });
    if let Some(retention) = &body.retention {
        params["retention"] = retention.clone();
    }
    let result: Value = state
        .core_call(methods::BACKUPS_SCHEDULE_CREATE, params)
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "backups.schedule.create",
        &body.name,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn schedule_delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::BACKUPS_SCHEDULE_DELETE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "backups.schedule.delete",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
