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
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let sites: Vec<Value> = state.core_call(methods::SITES_LIST, json!({})).await?;
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
