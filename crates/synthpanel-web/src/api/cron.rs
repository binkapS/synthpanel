//! Cron endpoints.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub expression: String,
    pub command: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobRequest {
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct DescribeQuery {
    pub expression: String,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let jobs: Vec<Value> = state.core_call(methods::CRON_LIST, json!({})).await?;
    Ok(Json(json!({ "jobs": jobs })))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateJobRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({
        "expression": body.expression,
        "command": body.command,
    });
    if let Some(value) = &body.user {
        params["user"] = json!(value);
    }
    if let Some(value) = &body.site_id {
        params["site_id"] = json!(value);
    }
    let result: Value = state.core_call(methods::CRON_CREATE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "cron.create",
        &body.command,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateJobRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({ "id": id });
    if let Some(value) = &body.expression {
        params["expression"] = json!(value);
    }
    if let Some(value) = &body.command {
        params["command"] = json!(value);
    }
    if let Some(value) = &body.user {
        params["user"] = json!(value);
    }
    if let Some(value) = &body.site_id {
        params["site_id"] = json!(value);
    }
    let result: Value = state.core_call(methods::CRON_UPDATE, params).await?;
    audit(&state, &user.user_id, "", "cron.update", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::CRON_DELETE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "cron.delete", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn run(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::CRON_RUN, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "cron.run", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn runs(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<Value>, ApiError> {
    let runs: Vec<Value> = state
        .core_call(
            methods::CRON_RUNS,
            json!({ "job_id": id, "limit": query.limit }),
        )
        .await?;
    Ok(Json(json!({ "runs": runs })))
}

pub async fn describe(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<DescribeQuery>,
) -> Result<Json<Value>, ApiError> {
    let description: Value = state
        .core_call(
            methods::CRON_DESCRIBE,
            json!({ "expression": query.expression }),
        )
        .await?;
    Ok(Json(description))
}
