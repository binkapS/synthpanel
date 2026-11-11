//! Git deployment endpoints.

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

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    pub site_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CreateRequest {
    pub site_id: String,
    pub repo_url: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub target_subdir: Option<String>,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub deploy_user: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateRequest {
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub target_subdir: Option<String>,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub deploy_user: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// List the deployments.
pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let deployments: Vec<Value> = state
        .core_call(
            methods::DEPLOYMENTS_LIST,
            json!({ "site_id": query.site_id }),
        )
        .await?;
    Ok(Json(json!({ "deployments": deployments })))
}

/// Create a deployment.
pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateRequest>,
) -> Result<Json<Value>, ApiError> {
    let deployment: Value = state
        .core_call(
            methods::DEPLOYMENTS_CREATE,
            json!({
                "site_id": body.site_id,
                "repo_url": body.repo_url,
                "branch": body.branch.unwrap_or_else(|| "main".into()),
                "target_subdir": body.target_subdir.unwrap_or_default(),
                "build_command": body.build_command.unwrap_or_default(),
                "deploy_user": body.deploy_user.unwrap_or_else(|| "www-data".into()),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "deployment.create",
        &body.repo_url,
        &user.ip,
    )
    .await;
    Ok(Json(deployment))
}

/// Update a deployment.
pub async fn update(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateRequest>,
) -> Result<Json<Value>, ApiError> {
    let deployment: Value = state
        .core_call(
            methods::DEPLOYMENTS_UPDATE,
            json!({
                "id": id,
                "repo_url": body.repo_url,
                "branch": body.branch,
                "target_subdir": body.target_subdir,
                "build_command": body.build_command,
                "deploy_user": body.deploy_user,
                "enabled": body.enabled,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "deployment.update",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(deployment))
}

/// Delete a deployment.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::DEPLOYMENTS_DELETE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "deployment.delete",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// Run a deployment.
pub async fn run(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let run: Value = state
        .core_call(methods::DEPLOYMENTS_RUN, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "deployment.run", &id, &user.ip).await;
    Ok(Json(run))
}

/// The run history of a deployment.
pub async fn runs(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let runs: Vec<Value> = state
        .core_call(methods::DEPLOYMENTS_RUNS, json!({ "id": id }))
        .await?;
    Ok(Json(json!({ "runs": runs })))
}
