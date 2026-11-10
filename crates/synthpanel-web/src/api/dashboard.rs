//! Dashboard data endpoints.

use crate::error::ApiError;
use crate::extract::AuthUser;
use std::sync::Arc;

use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde_json::{Value, json};
use std::collections::HashMap;
use synthpanel_ipc::methods;

pub async fn metrics_snapshot(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let snapshot: Option<Value> = state
        .core_call(methods::METRICS_SNAPSHOT, json!({}))
        .await?;
    Ok(Json(snapshot.unwrap_or(Value::Null)))
}

pub async fn metrics_range(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let range = query.get("range").cloned().unwrap_or_else(|| "24h".into());
    let rows: Vec<Value> = state
        .core_call(methods::METRICS_RANGE, json!({ "range": range }))
        .await?;
    Ok(Json(json!({ "range": range, "points": rows })))
}

pub async fn services(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let services: Vec<Value> = state.core_call(methods::SERVICES_LIST, json!({})).await?;
    Ok(Json(json!({ "services": services })))
}

pub async fn audit(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(25)
        .clamp(1, 200);
    let entries: Vec<Value> = state
        .core_call(methods::AUDIT_LIST, json!({ "limit": limit }))
        .await?;
    Ok(Json(json!({ "entries": entries })))
}
