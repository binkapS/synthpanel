//! Remote / external database server endpoints.
//!
//! Routes (registered by `api::mod::router`):
//!
//! - `GET    /api/v1/databases/remote`        -> `list`
//! - `POST   /api/v1/databases/remote`        -> `add`
//! - `DELETE /api/v1/databases/remote/{id}`   -> `delete`
//!
//! `list` wraps the IPC array in `{ "servers": [...] }`, matching the other
//! list endpoints in this API; `add` and `delete` forward the IPC result
//! (`{ "id": ... }` and `{ "ok": true }`). Passwords are sent to the core only
//! in the add request and are never echoed back.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;
use std::sync::Arc;
use synthpanel_ipc::methods;

/// Body of `POST /api/v1/databases/remote`.
///
/// `port` defaults to 3306 (MySQL/MariaDB) or 5432 (PostgreSQL) in the core,
/// and `tls` defaults to `false`.
#[derive(Deserialize)]
pub struct AddRemoteRequest {
    pub name: String,
    pub engine: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub tls: bool,
}

impl fmt::Debug for AddRemoteRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddRemoteRequest")
            .field("name", &self.name)
            .field("engine", &self.engine)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("tls", &self.tls)
            .finish()
    }
}

/// `GET /api/v1/databases/remote`
pub async fn list(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let servers: Vec<Value> = state
        .core_call(methods::DATABASES_REMOTE_LIST, json!({}))
        .await?;
    Ok(Json(json!({ "servers": servers })))
}

/// `POST /api/v1/databases/remote`
pub async fn add(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<AddRemoteRequest>,
) -> Result<Json<Value>, ApiError> {
    let params = json!({
        "name": body.name,
        "engine": body.engine,
        "host": body.host,
        "port": body.port,
        "database": body.database,
        "username": body.username,
        "password": body.password,
        "tls": body.tls,
    });
    let result: Value = state
        .core_call(methods::DATABASES_REMOTE_ADD, params)
        .await?;
    // Audit the server name, never the credential.
    audit(
        &state,
        &user.user_id,
        "",
        "databases.remote.add",
        &body.name,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// `DELETE /api/v1/databases/remote/{id}`
pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::DATABASES_REMOTE_DELETE, json!({ "id": id }))
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "databases.remote.delete",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}
