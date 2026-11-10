//! Log viewer endpoints.

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default = "default_source")]
    pub r#type: String,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub since_minutes: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_source() -> String {
    "panel".to_string()
}

fn default_limit() -> i64 {
    500
}

pub async fn sources(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let sources: Vec<Value> = state.core_call(methods::LOGS_SOURCES, json!({})).await?;
    Ok(Json(json!({ "sources": sources })))
}

pub async fn read(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<LogQuery>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::LOGS_READ,
            json!({
                "source": query.r#type,
                "site_id": query.site_id,
                "search": query.search,
                "status": query.status,
                "level": query.level,
                "since_minutes": query.since_minutes,
                "limit": query.limit,
            }),
        )
        .await?;
    Ok(Json(result))
}
