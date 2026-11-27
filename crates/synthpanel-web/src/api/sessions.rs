//! Browser session management endpoints.
//!
//! Sessions live in the web process, so these handlers only read and revoke
//! entries in the local store. They never talk to the core; in particular, a
//! session is not moved into the root core process.

use crate::api::auth::audit;
use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::sync::Arc;

/// Require a browser session: an API key has no session of its own, and
/// letting a key revoke browser sessions would be a privilege escalation.
fn browser_session(user: &AuthUser) -> Result<(), ApiError> {
    if user.api_key {
        Err(ApiError::forbidden(
            "session management requires a browser session",
        ))
    } else {
        Ok(())
    }
}

/// `GET /api/v1/auth/sessions`
pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    browser_session(&user)?;
    let sessions: Vec<Value> = state
        .sessions
        .list_for_user(&user.user_id, &user.token)
        .into_iter()
        .map(|session| {
            json!({
                "id": session.id,
                "ip": session.ip,
                "user_agent": session.user_agent,
                "age_seconds": session.age_seconds,
                "expires_in_seconds": session.expires_in_seconds,
                "current": session.current,
            })
        })
        .collect();
    Ok(Json(json!({
        "sessions": sessions,
        "count": sessions.len(),
    })))
}

/// `POST /api/v1/auth/sessions/revoke-others`
pub async fn revoke_others(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    browser_session(&user)?;
    let revoked = state.sessions.revoke_all_others(&user.user_id, &user.token);
    audit(
        &state,
        &user.user_id,
        "",
        "sessions.revoke_others",
        &revoked.to_string(),
        &user.ip,
    )
    .await;
    Ok(Json(json!({ "ok": true, "revoked": revoked })))
}
