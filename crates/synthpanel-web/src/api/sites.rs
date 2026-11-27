//! Site management endpoints.

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
pub struct CreateSiteRequest {
    pub domain: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub document_root: Option<String>,
    #[serde(default = "default_template")]
    pub template: String,
    #[serde(default)]
    pub php_version: Option<String>,
    #[serde(default = "default_ssl")]
    pub ssl_mode: String,
    #[serde(default)]
    pub directives: String,
    #[serde(default)]
    pub overrides: Value,
}

fn default_template() -> String {
    "php".to_string()
}

fn default_ssl() -> String {
    "none".to_string()
}

#[derive(Debug, Deserialize)]
pub struct UpdateSiteRequest {
    pub domain: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub document_root: Option<String>,
    pub template: Option<String>,
    pub php_version: Option<Value>, // null clears
    pub ssl_mode: Option<String>,
    pub status: Option<String>,
    pub directives: Option<String>,
    pub overrides: Option<Value>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let sites: Vec<Value> = state.core_call(methods::SITES_LIST, json!({})).await?;
    // A plain user may only see sites assigned to their account. Operators
    // and administrators see the full list, matching authz.rs.
    if user.role == "user" {
        let response: Value = state
            .core_call(methods::USER_SITES_LIST, json!({ "user_id": user.user_id }))
            .await?;
        let allowed = crate::authz::assigned_site_ids(&response);
        let sites = sites
            .into_iter()
            .filter(|site| {
                site.get("id")
                    .and_then(Value::as_str)
                    .map(|id| allowed.iter().any(|value| value == id))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        return Ok(Json(json!({ "sites": sites })));
    }
    Ok(Json(json!({ "sites": sites })))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let site: Value = state
        .core_call(methods::SITES_GET, json!({ "id": id }))
        .await?;
    if site.is_null() {
        return Err(ApiError::not_found("site not found"));
    }
    Ok(Json(site))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateSiteRequest>,
) -> Result<Json<Value>, ApiError> {
    let domain = validate_domain(&body.domain)?;
    let overrides = serde_json::to_string(&body.overrides)
        .map_err(|_| ApiError::bad_request("overrides must be a JSON object"))?;
    let result: Value = state
        .core_call(
            methods::SITES_CREATE,
            json!({
                "domain": domain,
                "aliases": body.aliases,
                "document_root": body.document_root,
                "template": body.template,
                "php_version": body.php_version,
                "ssl_mode": body.ssl_mode,
                "directives": body.directives,
                "overrides_json": overrides,
            }),
        )
        .await?;
    audit(&state, &user.user_id, "", "sites.create", &domain, &user.ip).await;
    Ok(Json(result))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateSiteRequest>,
) -> Result<Json<Value>, ApiError> {
    let mut params = json!({ "id": id });
    if let Some(value) = &body.domain {
        params["domain"] = json!(validate_domain(value)?);
    }
    if let Some(value) = &body.aliases {
        params["aliases"] = json!(value);
    }
    if let Some(value) = &body.document_root {
        params["document_root"] = json!(value);
    }
    if let Some(value) = &body.template {
        params["template"] = json!(value);
    }
    if let Some(value) = &body.php_version {
        params["php_version"] = value.clone();
    }
    if let Some(value) = &body.ssl_mode {
        params["ssl_mode"] = json!(value);
    }
    if let Some(value) = &body.status {
        params["status"] = json!(value);
    }
    if let Some(value) = &body.directives {
        params["directives"] = json!(value);
    }
    if let Some(value) = &body.overrides {
        params["overrides_json"] = json!(
            serde_json::to_string(value)
                .map_err(|_| { ApiError::bad_request("overrides must be serializable JSON") })?
        );
    }
    let result: Value = state.core_call(methods::SITES_UPDATE, params).await?;
    audit(&state, &user.user_id, "", "sites.update", &id, &user.ip).await;
    Ok(Json(result))
}

/// List the PHP versions the panel manages.
pub async fn php_versions(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let versions: Vec<String> = state.core_call(methods::PHP_VERSIONS, json!({})).await?;
    Ok(Json(json!({ "versions": versions })))
}

/// Report the PHP installation of this server.
pub async fn php_status(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state.core_call(methods::PHP_STATUS, json!({})).await?;
    Ok(Json(status))
}

/// Issue a Let's Encrypt certificate for a site over HTTP-01.
pub async fn issue_ssl(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::SITES_SSL_ISSUE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "sites.ssl.issue", &id, &user.ip).await;
    Ok(Json(result))
}

/// Report the certificate status of a site.
pub async fn ssl_status(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state
        .core_call(methods::SITES_SSL_STATUS, json!({ "id": id }))
        .await?;
    Ok(Json(status))
}

/// Suspend a site: it keeps serving a 503 page.
pub async fn suspend(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SITES_UPDATE,
            json!({ "id": id, "status": "suspended" }),
        )
        .await?;
    audit(&state, &user.user_id, "", "sites.suspend", &id, &user.ip).await;
    Ok(Json(result))
}

/// Bring a suspended site back online.
pub async fn unsuspend(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SITES_UPDATE,
            json!({ "id": id, "status": "active" }),
        )
        .await?;
    audit(&state, &user.user_id, "", "sites.unsuspend", &id, &user.ip).await;
    Ok(Json(result))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(methods::SITES_DELETE, json!({ "id": id }))
        .await?;
    audit(&state, &user.user_id, "", "sites.delete", &id, &user.ip).await;
    Ok(Json(result))
}

fn validate_domain(domain: &str) -> Result<String, ApiError> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err(ApiError::bad_request("a domain is required"));
    }
    if !is_valid_hostname(&domain) {
        return Err(ApiError::bad_request(format!("invalid domain `{domain}`")));
    }
    Ok(domain)
}

fn is_valid_hostname(value: &str) -> bool {
    if value.len() > 253 {
        return false;
    }
    value.split('.').all(|label| {
        let bytes = label.as_bytes();
        if bytes.is_empty() || bytes.len() > 63 {
            return false;
        }
        let first_ok = bytes[0].is_ascii_alphanumeric();
        let last_ok = bytes[bytes.len() - 1].is_ascii_alphanumeric();
        let interior_ok = bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-');
        first_ok && last_ok && interior_ok
    })
}

/// Per-site security settings.
#[derive(Debug, Deserialize)]
pub struct SiteSecurityRequest {
    #[serde(default)]
    pub ip_blocks: Vec<String>,
    #[serde(default)]
    pub bot_blocks: Vec<String>,
    #[serde(default)]
    pub basic_auth_enabled: bool,
    #[serde(default)]
    pub basic_auth_user: String,
    #[serde(default)]
    pub basic_auth_password: Option<String>,
    #[serde(default)]
    pub cloudflare_only: bool,
}

pub async fn security_get(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let settings: Value = state
        .core_call(methods::SITE_SECURITY_GET, json!({ "site_id": id }))
        .await?;
    Ok(Json(settings))
}

pub async fn security_set(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<SiteSecurityRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SITE_SECURITY_SET,
            json!({
                "site_id": id,
                "ip_blocks": request.ip_blocks,
                "bot_blocks": request.bot_blocks,
                "basic_auth_enabled": request.basic_auth_enabled,
                "basic_auth_user": request.basic_auth_user,
                "basic_auth_password": request.basic_auth_password,
                "cloudflare_only": request.cloudflare_only,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "sites.security.set",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct CertInstallRequest {
    pub cert: String,
    pub key: String,
    #[serde(default)]
    pub chain: Option<String>,
}

pub async fn cert_install(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CertInstallRequest>,
) -> Result<Json<Value>, ApiError> {
    let result: Value = state
        .core_call(
            methods::SSL_CERT_INSTALL,
            json!({
                "site_id": id,
                "cert": request.cert,
                "key": request.key,
                "chain": request.chain,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "sites.ssl.install",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// Configure a Node.js or Python app for a site.
pub async fn app_configure(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut params = body;
    if let serde_json::Value::Object(map) = &mut params {
        map.insert("site_id".into(), json!(id));
    }
    let result: Value = state
        .core_call(methods::SITES_APP_CONFIGURE, params)
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "sites.app.configure",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// Report the supervised app's status for a site.
pub async fn app_status(
    State(state): State<Arc<AppState>>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let status: Value = state
        .core_call(methods::SITES_APP_STATUS, json!({ "site_id": id }))
        .await?;
    Ok(Json(status))
}

pub async fn app_control(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("restart")
        .to_string();
    let result: Value = state
        .core_call(
            methods::SITES_APP_CONTROL,
            json!({ "site_id": id, "action": action }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "sites.app.control",
        &id,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_domains() {
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("www.example.com"));
        assert!(is_valid_hostname("sub.example.io"));
        assert!(!is_valid_hostname("-bad.example.com"));
        assert!(!is_valid_hostname("exa_mple.com"));
        assert!(!is_valid_hostname(""));
    }

    #[test]
    fn normalises_input() {
        assert_eq!(validate_domain(" Example.com ").unwrap(), "example.com");
        assert!(validate_domain("").is_err());
        assert!(validate_domain("bad_host").is_err());
    }
}
