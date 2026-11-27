//! On-demand database dump and import endpoints.

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
pub struct ImportDumpRequest {
    /// Base64-encoded dump (raw SQL text is accepted by the core as well).
    pub data: String,
}

/// `POST /api/v1/databases/{id}/dump`
pub async fn dump(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::DATABASES_DUMP, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "databases.dump", &id, &user.ip).await;
    Ok(Json(result))
}

/// `POST /api/v1/databases/{id}/import`
pub async fn import(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ImportDumpRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::DATABASES_IMPORT,
            json!({ "id": id, "data": body.data }),
        )
        .await?;
    audit(&state, &user.user_id, "", "databases.import", &id, &user.ip).await;
    Ok(Json(result))
}
