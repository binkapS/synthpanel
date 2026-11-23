//! File manager endpoints.

use crate::api::auth::audit;
use crate::authz::Policy;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

/// The scope that holds the configuration of the panel.
const PANEL_SCOPE: &str = "etc";

/// Check that the account may use this scope and path.
///
/// - The panel configuration holds the vhosts and the secrets of the panel:
///   only an administrator may read or write it.
/// - The web root holds the document roots of the sites: an account without
///   the operator role may only touch the sites it owns.
async fn check_scope(
    state: &AppState,
    user: &AuthUser,
    scope: &str,
    path: Option<&str>,
) -> Result<(), ApiError> {
    match scope {
        PANEL_SCOPE => {
            if !user.may(Policy::Admin) {
                return Err(ApiError::forbidden(
                    "the panel configuration is only available to an administrator",
                ));
            }
        }
        "www" | "" => {
            if user.may(Policy::Operator) {
                return Ok(());
            }
            let owned = owned_site_paths(state, &user.user_id).await?;
            let path = path.unwrap_or("").trim_start_matches('/');
            if !owned
                .iter()
                .any(|root| path == root || path.starts_with(&format!("{root}/")))
            {
                return Err(ApiError::forbidden("you may only manage your own sites"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// The document roots of the sites the account owns, relative to the web root.
async fn owned_site_paths(state: &AppState, user_id: &str) -> Result<Vec<String>, ApiError> {
    let sites: Vec<Value> = state.core_call(methods::SITES_LIST, json!({})).await?;
    let www = state.config.www_dir.to_string_lossy().to_string();
    let mut paths = Vec::new();
    for site in sites {
        if site.get("owner_id").and_then(Value::as_str) != Some(user_id) {
            continue;
        }
        let Some(root) = site.get("document_root").and_then(Value::as_str) else {
            continue;
        };
        let relative = root
            .strip_prefix(&www)
            .unwrap_or(root)
            .trim_start_matches('/')
            .to_string();
        if !relative.is_empty() {
            paths.push(relative);
        }
    }
    Ok(paths)
}

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
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, query.path.as_deref()).await?;
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
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query
        .path
        .ok_or_else(|| ApiError::bad_request("a path is required"))?;
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&path)).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path.clone())).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path.clone())).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.from)).await?;
    check_scope(&state, &user, &scope, Some(&body.to)).await?;
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
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, query.path.as_deref()).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
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
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
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
