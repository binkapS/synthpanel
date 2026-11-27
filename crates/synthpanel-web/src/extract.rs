//! Request extractors.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const SESSION_COOKIE: &str = "synthpanel_session";
pub const CSRF_COOKIE: &str = "synthpanel_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Authenticated user extracted from the session cookie or an API key.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub token: String,
    pub csrf: String,
    pub ip: String,
    /// The role of the account: `admin`, `operator` or `user`.
    pub role: String,
    /// The scopes of an API key; empty for a session.
    pub scopes: Vec<String>,
    /// Whether the request was authenticated with an API key.
    pub api_key: bool,
    /// Explicit site assignments of an API key. Empty means the key has no
    /// key-level restriction (legacy keys and all session users).
    pub site_ids: Vec<String>,
}

impl AuthUser {
    /// Whether the account may perform an operation of this policy.
    pub fn may(&self, policy: crate::authz::Policy) -> bool {
        if self.api_key {
            self.scopes.iter().any(|scope| scope == policy.scope())
                || (policy == crate::authz::Policy::Read && !self.scopes.is_empty())
        } else {
            policy.allows(&self.role)
        }
    }
}

/// Read the explicit site assignments from an `api_keys.verify` response.
///
/// The documented shape is an array of site ids. Records carrying `site_id`
/// (or `id`) are accepted too; a missing or malformed field yields no
/// restriction, which keeps pre-wave-4 keys working.
pub fn api_key_site_ids(record: &Value) -> Vec<String> {
    let Some(entries) = record.get("site_ids").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = Vec::new();
    for entry in entries {
        let id = match entry {
            Value::String(id) => Some(id.as_str()),
            Value::Object(_) => entry
                .get("site_id")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str),
            _ => None,
        };
        if let Some(id) = id {
            let id = id.trim();
            if !id.is_empty() && !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// Enforcement helper for site-scoped requests.
///
/// `key_site_ids` are the API key's explicit assignments (empty = no
/// key-level restriction). `owner_site_ids` are the account's grants from the
/// core's `user_sites` table. An explicit key assignment must contain the
/// requested site, and the role check still applies: a key owned by a `user`
/// can only reach the intersection of both lists. Unknown roles and empty
/// site ids get no access.
pub fn api_key_site_allowed(
    role: &str,
    site_id: &str,
    key_site_ids: &[String],
    owner_site_ids: &[String],
) -> bool {
    if site_id.is_empty() {
        return false;
    }
    if !key_site_ids.is_empty() && !key_site_ids.iter().any(|id| id == site_id) {
        return false;
    }
    match role {
        "admin" | "operator" => true,
        "user" => owner_site_ids.iter().any(|id| id == site_id),
        _ => false,
    }
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
        // The authorization middleware resolves the actor once and puts it
        // into the extensions; the extractor reuses it.
        if let Some(context) = parts.extensions.get::<crate::authz::AuthContext>() {
            return Ok(AuthUser {
                user_id: context.user_id.clone(),
                token: context.token.clone(),
                csrf: context.csrf.clone(),
                ip: context.ip.clone(),
                role: context.role.clone(),
                scopes: context.scopes.clone(),
                api_key: context.api_key,
                // `AuthContext` is owned by `authz.rs`; once it carries the
                // key's assignments, pass them through here as well. Today
                // this extension is not populated by the middleware, so the
                // API key branch below is the one that sees real keys.
                site_ids: Vec::new(),
            });
        }

        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|connect| connect.0);
        let ip = crate::authz::client_ip(&parts.headers, peer);

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
                let scopes = record
                    .get("scopes")
                    .and_then(serde_json::Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                return Ok(AuthUser {
                    user_id: user_id.to_string(),
                    token: token.trim().to_string(),
                    csrf: String::new(),
                    ip,
                    role: record
                        .get("role")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("user")
                        .to_string(),
                    scopes,
                    api_key: true,
                    site_ids: api_key_site_ids(&record),
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
            role: record.role,
            scopes: Vec::new(),
            api_key: false,
            site_ids: Vec::new(),
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
    use serde_json::json;

    #[test]
    fn parses_cookie_headers() {
        let header = "synthpanel_session=abc; synthpanel_csrf=def; Path=/";
        assert_eq!(cookie_value(header, SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie_value(header, CSRF_COOKIE).as_deref(), Some("def"));
        assert!(cookie_value(header, "missing").is_none());
    }

    #[test]
    fn api_key_site_ids_are_parsed_defensively() {
        let cases = [
            (json!({}), vec![]),
            (json!({"site_ids": null}), vec![]),
            (json!({"site_ids": "site-1"}), vec![]),
            (
                json!({"site_ids": ["site-1", " site-2 ", "site-1", "", 3]}),
                vec!["site-1", "site-2"],
            ),
            (
                json!({"site_ids": [{"site_id": "site-3"}, {"id": "site-4"}]}),
                vec!["site-3", "site-4"],
            ),
        ];
        for (record, expected) in cases {
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert_eq!(api_key_site_ids(&record), expected, "record {record}");
        }
    }

    #[test]
    fn api_key_site_check_intersects_key_and_owner_sites() {
        let owned = vec!["site-1".to_string(), "site-2".to_string()];
        let restricted = vec!["site-1".to_string()];
        let other = vec!["site-9".to_string()];
        let cases = [
            // role, site, key sites, owner sites, expected
            ("admin", "site-1", vec![], owned.clone(), true),
            ("operator", "site-2", vec![], owned.clone(), true),
            ("user", "site-1", vec![], owned.clone(), true),
            ("user", "site-9", vec![], owned.clone(), false),
            // Explicit key sites limit even privileged roles.
            ("admin", "site-1", restricted.clone(), owned.clone(), true),
            ("admin", "site-2", restricted.clone(), owned.clone(), false),
            (
                "operator",
                "site-9",
                restricted.clone(),
                owned.clone(),
                false,
            ),
            // A user key is limited to the intersection of both lists.
            ("user", "site-1", restricted.clone(), owned.clone(), true),
            ("user", "site-2", restricted.clone(), owned.clone(), false),
            ("user", "site-9", other.clone(), owned.clone(), false),
            // An explicit but unmatched assignment never widens access.
            ("user", "site-1", other.clone(), vec![], false),
            ("admin", "", vec![], owned.clone(), false),
            ("guest", "site-1", vec![], owned.clone(), false),
            ("", "site-1", vec![], owned, false),
        ];
        for (role, site, key_sites, owner_sites, expected) in cases {
            assert_eq!(
                api_key_site_allowed(role, site, &key_sites, &owner_sites),
                expected,
                "role `{role}`, site `{site}`, key sites {key_sites:?}, owner sites {owner_sites:?}"
            );
        }
    }
}
