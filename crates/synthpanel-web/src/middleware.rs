//! Security middleware: response headers and CSRF protection.

use crate::error::ApiError;
use crate::extract::{CSRF_COOKIE, CSRF_HEADER, SESSION_COOKIE, cookie_value};
use std::sync::Arc;

use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     base-uri 'self'; \
     object-src 'none'; \
     frame-ancestors 'none'; \
     form-action 'self'; \
     img-src 'self' data:; \
     font-src 'self' data:; \
     style-src 'self' 'unsafe-inline'; \
     script-src 'self'; \
     connect-src 'self' ws: wss:";

/// Endpoints that are reachable without a session (and therefore without a
/// CSRF token).
const CSRF_EXEMPT: &[&str] = &["/api/v1/auth/login", "/api/v1/auth/2fa/verify"];

/// Add hardening headers to every response.
pub async fn security_headers(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=(), payment=()"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    if state.config.secure_cookies {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}

/// Double-submit CSRF check for state-changing requests.
pub async fn csrf_guard(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    if !matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || CSRF_EXEMPT.contains(&path.as_str())
    {
        return next.run(request).await;
    }

    // Bearer-token API clients are not vulnerable to CSRF: the token is never
    // attached automatically by a browser.
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("Bearer "))
        .unwrap_or(false)
    {
        return next.run(request).await;
    }

    let session_token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, SESSION_COOKIE));
    let provided = request
        .headers()
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let cookie_csrf = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, CSRF_COOKIE));

    let (Some(session_token), Some(provided)) = (session_token, provided) else {
        return ApiError::forbidden("missing CSRF token").into_response();
    };
    if let Some(cookie_csrf) = cookie_csrf {
        let match_cookie: bool = cookie_csrf.as_bytes().ct_eq(provided.as_bytes()).into();
        if !match_cookie {
            return ApiError::forbidden("invalid CSRF token").into_response();
        }
    }
    let Some(record) = state.sessions.get(&session_token) else {
        return ApiError::unauthorized("authentication required").into_response();
    };
    let match_session: bool = record.csrf.as_bytes().ct_eq(provided.as_bytes()).into();
    if !match_session {
        return ApiError::forbidden("invalid CSRF token").into_response();
    }
    next.run(request).await
}
