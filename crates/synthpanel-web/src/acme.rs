//! The ACME challenge endpoint.
//!
//! Let's Encrypt validates a domain over HTTP-01: it fetches
//! `http://<domain>/.well-known/acme-challenge/<token>` and expects the file
//! that acme.sh wrote. The panel serves that directory so it can request a
//! certificate for its own hostname.

use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Serve a challenge file.
pub async fn challenge(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    // Only the file name is allowed: no directories, no traversal.
    if token.is_empty()
        || token.contains('/')
        || token.contains('\\')
        || token.contains("..")
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = state.config.acme_webroot().join(&token);
    match tokio::fs::read(&path).await {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            Body::from(body),
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    /// The token check from the handler.
    fn is_unsafe(token: &str) -> bool {
        token.is_empty()
            || token.contains('/')
            || token.contains('\\')
            || token.contains("..")
            || !token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    #[test]
    fn rejects_unsafe_tokens() {
        for token in ["../secret", "a/b", "..", "", "a b"] {
            assert!(is_unsafe(token), "{token} should be rejected");
        }
        assert!(!is_unsafe("token-123_abc"));
    }
}
