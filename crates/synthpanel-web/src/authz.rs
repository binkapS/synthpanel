//! Authorization: what a route requires and who may call it.
//!
//! The panel has three roles and three levels of access. Every route belongs
//! to exactly one level, and the level is enforced by a middleware, so a new
//! route cannot be added without a policy: the default is the strictest one
//! for its area.
//!
//! | Area | admin | operator | user |
//! |---|---|---|---|
//! | terminal, settings, users, updates, security, alerts, panel | ✔ | ✘ | ✘ |
//! | sites, databases, files, backups, cron, deployments | ✔ | ✔ | ✘ |
//! | everything else (read-only) | ✔ | ✔ | ✔ |
//!
//! The middleware also resolves the actor of the request, so that the core
//! can attribute its audit entries to a person, and it refuses to trust the
//! forwarded headers unless the panel is explicitly configured to sit behind a
//! trusted proxy.

use crate::error::ApiError;
use crate::extract::{
    AuthUser, SESSION_COOKIE, api_key_site_allowed, api_key_site_ids, cookie_value,
};
use crate::state::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use synthpanel_ipc::{Actor, methods};

/// What a route requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Any authenticated account may call it.
    Read,
    /// An administrator or an operator may call it.
    Operator,
    /// Only an administrator may call it.
    Admin,
    /// A site-scoped operation.
    ///
    /// The middleware cannot enforce this policy on its own: it does not know
    /// which account may see which site. A handler with this policy must call
    /// [`require_site_access`] with the site id from its path. An API key
    /// needs the `write` scope because site-scoped routes change state.
    Site,
}

impl Policy {
    /// The scope an API key needs for this policy.
    pub fn scope(self) -> &'static str {
        match self {
            Policy::Read => "read",
            Policy::Operator | Policy::Site => "write",
            Policy::Admin => "admin",
        }
    }

    /// Whether the role may call a route with this policy.
    ///
    /// Every role can reach a site-scoped route in principle; the per-site
    /// assignment is checked separately by [`require_site_access`].
    pub fn allows(self, role: &str) -> bool {
        match self {
            Policy::Read | Policy::Site => true,
            Policy::Operator => matches!(role, "admin" | "operator"),
            Policy::Admin => role == "admin",
        }
    }
}

/// The areas that only an administrator may touch.
const ADMIN_AREAS: &[&str] = &[
    "/api/v1/terminal",
    "/api/v1/users",
    "/api/v1/security",
    "/api/v1/alerts",
    "/api/v1/update",
    "/api/v1/panel",
    "/api/v1/settings",
];

/// The areas an operator may work in.
const OPERATOR_AREAS: &[&str] = &[
    "/api/v1/sites",
    "/api/v1/databases",
    "/api/v1/backups",
    "/api/v1/cron",
    "/api/v1/deployments",
    "/api/v1/files",
    "/api/v1/api-keys",
];

/// The routes that are reachable without a session.
const PUBLIC_PATHS: &[&str] = &["/api/v1/auth/login", "/api/v1/auth/2fa/verify"];

/// The policy of a route.
///
/// The method matters: a `GET` is a read, everything else changes something.
pub fn policy_for(method: &Method, path: &str) -> Policy {
    if ADMIN_AREAS
        .iter()
        .any(|area| path == *area || path.starts_with(&format!("{area}/")))
    {
        return Policy::Admin;
    }
    // A concrete site is identified by /api/v1/sites/{id}/... . Those routes
    // are site-scoped: every role may reach them, but the handler must call
    // require_site_access with the id from the path so a `user` is limited to
    // their assigned sites. Creating/listing sites stays operator-only.
    if let Some(rest) = path.strip_prefix("/api/v1/sites/") {
        if !rest.is_empty() {
            return Policy::Site;
        }
    }
    let mutating = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if mutating
        && OPERATOR_AREAS
            .iter()
            .any(|area| path == *area || path.starts_with(&format!("{area}/")))
    {
        return Policy::Operator;
    }
    Policy::Read
}

/// Who is behind the request.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub name: String,
    pub role: String,
    pub csrf: String,
    pub token: String,
    pub ip: String,
    /// The scopes of an API key; empty for a session.
    pub scopes: Vec<String>,
    /// Whether the request was authenticated with an API key.
    pub api_key: bool,
    /// Explicit site assignments of an API key; empty means no key-level limit.
    pub site_ids: Vec<String>,
}

impl AuthContext {
    /// The actor as the IPC protocol wants it.
    pub fn actor(&self) -> Actor {
        Actor {
            id: self.user_id.clone(),
            name: self.name.clone(),
            ip: self.ip.clone(),
            scopes: self.scopes.clone(),
        }
    }
}

// The actor of the current request.
//
// The authorization middleware sets it, so the IPC client can attribute the
// request to a person without every handler having to pass it along.
tokio::task_local! {
    pub static ACTOR: Actor;
}

/// The current actor, if the request had one.
pub fn current_actor() -> Option<Actor> {
    ACTOR.try_with(|actor| actor.clone()).ok()
}

/// Read the site ids out of a `user_sites.list` response.
///
/// The core returns the assignments either as a list of ids or as a list of
/// site records; an object wrapping either shape is accepted as well. Any
/// other shape yields no sites, so a malformed answer cannot widen access.
pub fn assigned_site_ids(value: &Value) -> Vec<String> {
    let entries: Vec<&Value> = match value {
        Value::Array(list) => list.iter().collect(),
        Value::Object(map) => match map
            .get("sites")
            .or_else(|| map.get("site_ids"))
            .or_else(|| map.get("assignments"))
        {
            Some(Value::Array(list)) => list.iter().collect(),
            Some(record @ Value::Object(_)) => vec![record],
            _ => Vec::new(),
        },
        _ => Vec::new(),
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
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            if !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// The pure part of the site check, kept separate so it can be tested without
/// a running core.
///
/// Administrators and operators may work on every site; the operator's
/// exclusion from the administrative area is enforced by [`policy_for`]. A
/// plain `user` may only reach assigned sites, and an unknown role gets no
/// access at all.
pub fn site_access_allowed(role: &str, site_id: &str, assigned: &[String]) -> bool {
    if site_id.is_empty() {
        return false;
    }
    match role {
        "admin" | "operator" => true,
        "user" => assigned.iter().any(|id| id == site_id),
        _ => false,
    }
}

/// Require that the account may act on `site_id`.
///
/// Call this in every site-scoped handler (including reads). For an account
/// with the `user` role the assignments come from the core's `user_sites`
/// table; for an operator or administrator no IPC call is needed. API keys
/// inherit the role of their account, so a key owned by a `user` is limited
/// to that user's sites too.
pub async fn require_site_access(
    state: &AppState,
    user: &AuthUser,
    site_id: &str,
) -> Result<(), ApiError> {
    let assigned = match user.role.as_str() {
        "admin" | "operator" => Vec::new(),
        "user" => {
            let response: Value = state
                .core_call(
                    methods::USER_SITES_LIST,
                    serde_json::json!({ "user_id": user.user_id }),
                )
                .await?;
            assigned_site_ids(&response)
        }
        _ => Vec::new(),
    };

    if api_key_site_allowed(&user.role, site_id, &user.site_ids, &assigned) {
        Ok(())
    } else {
        Err(ApiError::forbidden("the account may not access this site"))
    }
}

/// The middleware-level variant of [`require_site_access`]. It uses the
/// resolved `AuthContext` so the check happens before the handler runs.
pub async fn require_site_access_ctx(
    state: &AppState,
    context: &AuthContext,
    site_id: &str,
) -> Result<(), ApiError> {
    let assigned = match context.role.as_str() {
        "admin" | "operator" => Vec::new(),
        "user" => {
            let response: Value = state
                .core_call(
                    methods::USER_SITES_LIST,
                    serde_json::json!({ "user_id": context.user_id }),
                )
                .await?;
            assigned_site_ids(&response)
        }
        _ => Vec::new(),
    };
    if api_key_site_allowed(&context.role, site_id, &context.site_ids, &assigned) {
        Ok(())
    } else {
        Err(ApiError::forbidden("the account may not access this site"))
    }
}

/// The address of the client.
///
/// The forwarded headers are only honoured when the panel is explicitly
/// configured to sit behind a trusted proxy: otherwise any client could
/// forge its own address in the audit log.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if crate::config::WebConfig::from_env().trusts_proxies() {
        if let Some(value) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            if let Some(first) = value.split(',').next() {
                let first = first.trim();
                if !first.is_empty() {
                    return first.to_string();
                }
            }
        }
    }
    peer.map(|peer| peer.ip().to_string()).unwrap_or_default()
}

/// Resolve the actor of the request.
///
/// `Ok(None)` means that the request carries no credentials at all; the
/// handler then answers with `401` on its own.
pub async fn resolve_actor(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<Option<AuthContext>, ApiError> {
    let ip = client_ip(headers, peer);

    // An API key is explicit and is never attached automatically by a
    // browser, so it bypasses the CSRF check.
    if let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            let token = token.trim();
            let record: serde_json::Value = state
                .core_call(
                    synthpanel_ipc::methods::API_KEYS_VERIFY,
                    serde_json::json!({ "token": token }),
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
            let name = record
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Ok(Some(AuthContext {
                user_id: user_id.to_string(),
                name,
                role: record
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("user")
                    .to_string(),
                csrf: String::new(),
                token: token.to_string(),
                ip,
                scopes,
                api_key: true,
                site_ids: api_key_site_ids(&record),
            }));
        }
    }

    let token = headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, SESSION_COOKIE));

    let Some(token) = token else {
        return Ok(None);
    };
    let Some(record) = state.sessions.get(&token) else {
        return Ok(None);
    };

    Ok(Some(AuthContext {
        user_id: record.user_id.clone(),
        name: record.name.clone(),
        role: record.role.clone(),
        csrf: record.csrf.clone(),
        token,
        ip: if record.ip.is_empty() { ip } else { record.ip },
        scopes: Vec::new(),
        api_key: false,
        site_ids: Vec::new(),
    }))
}

/// The middleware that enforces the policy.
pub async fn authorize(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    // The headers are copied out of the request: the request itself may not
    // be held across an await, because its body is not `Sync`.
    let headers = request.headers().clone();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0);

    if PUBLIC_PATHS.contains(&path.as_str()) {
        return next.run(request).await;
    }

    let context = match resolve_actor(&state, &headers, peer).await {
        Ok(Some(context)) => context,
        Ok(None) => {
            // No credentials: the handler answers with `401`.
            return next.run(request).await;
        }
        Err(error) => return error.into_response(),
    };

    let policy = policy_for(&method, &path);
    let allowed = if context.api_key {
        context.scopes.iter().any(|scope| scope == policy.scope())
            || (policy == Policy::Read && !context.scopes.is_empty())
    } else {
        policy.allows(&context.role)
    };

    if !allowed {
        // The rejection itself is worth an audit entry: it is the signal an
        // operator looks for.
        let action = format!("authorization.denied.{method}");
        crate::api::auth::audit_actor(
            &state,
            &context.user_id,
            &context.name,
            &action,
            &path,
            &context.ip,
        )
        .await;
        return ApiError::forbidden("the account may not perform this operation").into_response();
    }

    if policy == Policy::Site {
        let site_id = path
            .strip_prefix("/api/v1/sites/")
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default();
        if let Err(error) = require_site_access_ctx(&state, &context, site_id).await {
            return error.into_response();
        }
    }

    let actor = context.actor();
    ACTOR.scope(actor, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get(path: &str) -> Policy {
        policy_for(&Method::GET, path)
    }

    fn post(path: &str) -> Policy {
        policy_for(&Method::POST, path)
    }

    fn patch(path: &str) -> Policy {
        policy_for(&Method::PATCH, path)
    }

    #[test]
    fn the_admin_areas_are_admin_only() {
        assert_eq!(get("/api/v1/terminal"), Policy::Admin);
        assert_eq!(post("/api/v1/users"), Policy::Admin);
        assert_eq!(post("/api/v1/security/ssh-keys"), Policy::Admin);
        assert_eq!(post("/api/v1/alerts/channels"), Policy::Admin);
        assert_eq!(post("/api/v1/update/apply"), Policy::Admin);
        assert_eq!(post("/api/v1/panel/ssl/issue"), Policy::Admin);
        assert_eq!(patch("/api/v1/settings"), Policy::Admin);
    }

    #[test]
    fn the_operator_areas_need_an_operator_to_change_them() {
        assert_eq!(post("/api/v1/cron"), Policy::Operator);
        assert_eq!(post("/api/v1/sites"), Policy::Operator);
        assert_eq!(post("/api/v1/deployments"), Policy::Operator);
        assert_eq!(patch("/api/v1/files/content"), Policy::Operator);
        // …but everybody may read them.
        assert_eq!(get("/api/v1/sites"), Policy::Read);
        assert_eq!(get("/api/v1/cron"), Policy::Read);
        assert_eq!(get("/api/v1/backups"), Policy::Read);
    }

    #[test]
    fn the_unmapped_routes_are_read_only() {
        assert_eq!(get("/api/v1/metrics/snapshot"), Policy::Read);
        assert_eq!(get("/api/v1/logs"), Policy::Read);
        assert_eq!(get("/api/v1/audit"), Policy::Read);
    }

    #[test]
    fn the_roles_map_onto_the_policies() {
        assert!(Policy::Read.allows("user"));
        assert!(!Policy::Operator.allows("user"));
        assert!(Policy::Operator.allows("operator"));
        assert!(Policy::Operator.allows("admin"));
        assert!(!Policy::Admin.allows("operator"));
        assert!(Policy::Admin.allows("admin"));
    }

    #[test]
    fn the_scopes_map_onto_the_policies() {
        assert_eq!(Policy::Read.scope(), "read");
        assert_eq!(Policy::Operator.scope(), "write");
        assert_eq!(Policy::Admin.scope(), "admin");
        assert_eq!(Policy::Site.scope(), "write");
    }

    #[test]
    fn the_site_policy_delegates_the_check_to_the_handler() {
        assert!(Policy::Site.allows("admin"));
        assert!(Policy::Site.allows("operator"));
        assert!(Policy::Site.allows("user"));
    }

    #[test]
    fn site_access_is_decided_by_the_role_and_the_assignments() {
        let cases = [
            ("admin", "site-1", vec![], true),
            ("operator", "site-9", vec![], true),
            ("admin", "", vec![], false),
            ("operator", "", vec![], false),
            ("user", "site-1", vec!["site-1".into()], true),
            ("user", "site-2", vec!["site-1".into()], false),
            ("user", "site-1", vec![], false),
            ("user", "", vec!["".into()], false),
            ("guest", "site-1", vec!["site-1".into()], false),
            ("", "site-1", vec!["site-1".into()], false),
        ];
        for (role, site, assigned, expected) in cases {
            assert_eq!(
                site_access_allowed(role, site, &assigned),
                expected,
                "role `{role}`, site `{site}`, assignments {assigned:?}"
            );
        }
    }

    #[test]
    fn assignments_are_read_from_every_supported_shape() {
        let cases = [
            (json!(["site-1", "site-2"]), vec!["site-1", "site-2"]),
            (
                json!([{"id": "site-1"}, {"id": "site-2"}]),
                vec!["site-1", "site-2"],
            ),
            (json!([{"site_id": "site-1"}]), vec!["site-1"]),
            (json!({"sites": ["site-1"]}), vec!["site-1"]),
            (json!({"site_ids": ["site-1", "site-1"]}), vec!["site-1"]),
            (json!({"assignments": [{"id": "site-3"}]}), vec!["site-3"]),
            (
                json!({"sites": [{"id": "site-4", "role": "user"}]}),
                vec!["site-4"],
            ),
            (json!([]), vec![]),
            (json!({}), vec![]),
            (json!(null), vec![]),
            (json!("site-1"), vec![]),
            (json!({"sites": "site-1"}), vec![]),
            (json!([{"id": ""}, 42, null]), vec![]),
        ];
        for (value, expected) in cases {
            let ids = assigned_site_ids(&value);
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert_eq!(ids, expected, "response {value}");
        }
    }
}
