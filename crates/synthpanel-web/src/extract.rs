//! Request extractors.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

pub const SESSION_COOKIE: &str = "synthpanel_session";
pub const CSRF_COOKIE: &str = "synthpanel_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Authenticated user extracted from the session cookie.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub token: String,
    pub csrf: String,
    pub ip: String,
}

/// Parse a `Cookie:` header into `(name, value)` pairs.
pub fn parse_cookies(header: &str) -> Vec<(String, String)> {
    header
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            part.split_once('=')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    parse_cookies(header)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

impl FromRequestParts<std::sync::Arc<AppState>> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &std::sync::Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let ip = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(|value| value.trim().to_string())
            .unwrap_or_default();

        // API keys take precedence: a bearer token is explicit and is never
        // sent automatically by a browser, so CSRF does not apply.
        if let Some(authorization) = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            if let Some(token) = authorization.strip_prefix("Bearer ") {
                let record: serde_json::Value = state
                    .core_call(
                        synthpanel_ipc::methods::API_KEYS_VERIFY,
                        serde_json::json!({ "token": token.trim() }),
                    )
                    .await
                    .map_err(|_| ApiError::unauthorized("invalid API key"))?;
                let user_id = record
                    .get("user_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| ApiError::unauthorized("invalid API key"))?;
                return Ok(AuthUser {
                    user_id: user_id.to_string(),
                    token: token.trim().to_string(),
                    csrf: String::new(),
                    ip,
                });
            }
        }

        let token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| cookie_value(value, SESSION_COOKIE));

        let Some(token) = token else {
            tracing::warn!(
                path = %parts.uri.path(),
                ip = %ip,
                "the request carries no session cookie"
            );
            return Err(ApiError::unauthorized("authentication required"));
        };

        let record = state.sessions.get(&token);
        let Some(record) = record else {
            tracing::warn!(
                path = %parts.uri.path(),
                ip = %ip,
                "the session is unknown or has expired"
            );
            return Err(ApiError::unauthorized("session expired or invalid"));
        };

        Ok(AuthUser {
            user_id: record.user_id,
            token,
            csrf: record.csrf,
            ip: record.ip,
        })
    }
}

/// JSON body extractor that reports failures using the API error format.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(ApiError::bad_request(rejection.body_text())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_headers() {
        let header = "synthpanel_session=abc; synthpanel_csrf=def; Path=/";
        assert_eq!(cookie_value(header, SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie_value(header, CSRF_COOKIE).as_deref(), Some("def"));
        assert!(cookie_value(header, "missing").is_none());
    }
}
