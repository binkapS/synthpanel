//! File manager endpoints.

use crate::api::auth::audit;
use crate::authz::Policy;
use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser};
use crate::state::AppState;
use axum::Json;
use axum::body::to_bytes;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use base64::Engine;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use synthpanel_ipc::methods;

/// The scope that holds the configuration of the panel.
const PANEL_SCOPE: &str = "etc";

/// The largest upload the endpoint accepts (8 MiB).
const UPLOAD_LIMIT: usize = 8 * 1024 * 1024;

/// Check that the account may use this scope and path.
///
/// - The panel configuration holds the vhosts and the secrets of the panel:
///   only an administrator may read or write it.
/// - The web root holds the document roots of the sites: an account without
///   the operator role may only touch the sites it owns.
async fn check_scope(
    state: &AppState,
    user: &AuthUser,
    scope: &str,
    path: Option<&str>,
) -> Result<(), ApiError> {
    match scope {
        PANEL_SCOPE => {
            if !user.may(Policy::Admin) {
                return Err(ApiError::forbidden(
                    "the panel configuration is only available to an administrator",
                ));
            }
        }
        "www" | "" => {
            if user.may(Policy::Operator) {
                return Ok(());
            }
            let owned = owned_site_paths(state, &user.user_id).await?;
            let path = path.unwrap_or("").trim_start_matches('/');
            if !owned
                .iter()
                .any(|root| path == root || path.starts_with(&format!("{root}/")))
            {
                return Err(ApiError::forbidden("you may only manage your own sites"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// The document roots of the sites the account owns, relative to the web root.
async fn owned_site_paths(state: &AppState, user_id: &str) -> Result<Vec<String>, ApiError> {
    let sites: Vec<Value> = state.core_call(methods::SITES_LIST, json!({})).await?;
    let www = state.config.www_dir.to_string_lossy().to_string();
    let mut paths = Vec::new();
    for site in sites {
        if site.get("owner_id").and_then(Value::as_str) != Some(user_id) {
            continue;
        }
        let Some(root) = site.get("document_root").and_then(Value::as_str) else {
            continue;
        };
        let relative = root
            .strip_prefix(&www)
            .unwrap_or(root)
            .trim_start_matches('/')
            .to_string();
        if !relative.is_empty() {
            paths.push(relative);
        }
    }
    Ok(paths)
}

#[derive(Debug, Deserialize, Default)]
pub struct PathQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MkdirRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CopyRequest {
    pub path: String,
    pub target: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChmodRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ArchiveRequest {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub path: String,
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, query.path.as_deref()).await?;
    let result: Value = state
        .core_call(
            methods::FILES_LIST,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": query.path.unwrap_or_default(),
            }),
        )
        .await?;
    Ok(Json(result))
}

pub async fn read(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query
        .path
        .ok_or_else(|| ApiError::bad_request("a path is required"))?;
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&path)).await?;
    let result: Value = state
        .core_call(
            methods::FILES_READ,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": path,
            }),
        )
        .await?;
    Ok(Json(result))
}

pub async fn write(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<WriteRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path.clone())).await?;
    let mut params = json!({
        "scope": body.scope.unwrap_or_else(|| "www".into()),
        "path": body.path,
    });
    if let Some(content) = &body.content {
        params["content"] = json!(content);
    }
    if let Some(encoded) = &body.content_base64 {
        params["content_base64"] = json!(encoded);
    }
    let result: Value = state.core_call(methods::FILES_WRITE, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.write",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn mkdir(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<MkdirRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path.clone())).await?;
    let result: Value = state
        .core_call(
            methods::FILES_MKDIR,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.mkdir",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<RenameRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.from)).await?;
    check_scope(&state, &user, &scope, Some(&body.to)).await?;
    let result: Value = state
        .core_call(
            methods::FILES_RENAME,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "from": body.from,
                "to": body.to,
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.rename",
        &body.from,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn copy(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<CopyRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    let path = body.path.clone();
    let target = body.target.clone();
    check_scope(&state, &user, &scope, Some(&path)).await?;
    check_scope(&state, &user, &scope, Some(&target)).await?;
    let result: Value = state
        .core_call(
            methods::FILES_COPY,
            json!({
                "scope": scope,
                "path": path,
                "target": target,
            }),
        )
        .await?;
    audit(&state, &user.user_id, "", "files.copy", &path, &user.ip).await;
    Ok(Json(result))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let scope = query.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, query.path.as_deref()).await?;
    let path = query
        .path
        .ok_or_else(|| ApiError::bad_request("a path is required"))?;
    let result: Value = state
        .core_call(
            methods::FILES_DELETE,
            json!({
                "scope": query.scope.unwrap_or_else(|| "www".into()),
                "path": path,
            }),
        )
        .await?;
    audit(&state, &user.user_id, "", "files.delete", &path, &user.ip).await;
    Ok(Json(result))
}

pub async fn chmod(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ChmodRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
    let mut params = json!({
        "scope": body.scope.unwrap_or_else(|| "www".into()),
        "path": body.path,
    });
    if let Some(mode) = &body.mode {
        params["mode"] = json!(mode);
    }
    if let Some(owner) = &body.owner {
        params["owner"] = json!(owner);
    }
    if let Some(group) = &body.group {
        params["group"] = json!(group);
    }
    let result: Value = state.core_call(methods::FILES_CHMOD, params).await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.chmod",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn archive(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ArchiveRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
    let result: Value = state
        .core_call(
            methods::FILES_ARCHIVE,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
                "format": body.format.unwrap_or_else(|| "tar.gz".into()),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.archive",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

pub async fn extract(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(body): ApiJson<ExtractRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope = body.scope.clone().unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&body.path)).await?;
    let result: Value = state
        .core_call(
            methods::FILES_EXTRACT,
            json!({
                "scope": body.scope.unwrap_or_else(|| "www".into()),
                "path": body.path,
                "into": body.into.unwrap_or_default(),
            }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        "",
        "files.extract",
        &body.path,
        &user.ip,
    )
    .await;
    Ok(Json(result))
}

/// Upload a file by forwarding its bytes to `FILES_WRITE`.
///
/// Three bodies are accepted:
///
/// - `application/json` with `scope`, `path` and either `content` or
///   `content_base64` (this is what the UI bridge sends);
/// - `multipart/form-data` with `scope`, `path` and a `file` part;
/// - any other content type, treated as the raw file body with `scope` and
///   `path` taken from the query string.
///
/// Session, authorization and CSRF checks are the same as for every other
/// file route: they are applied by the shared middleware and `check_scope`.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(query): Query<PathQuery>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let content_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = to_bytes(request.into_body(), UPLOAD_LIMIT)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "the file is too large",
            )
        })?;

    let (scope, path, content) = if content_type.starts_with("multipart/form-data") {
        let boundary = multipart_boundary(&content_type)
            .ok_or_else(|| ApiError::bad_request("the multipart boundary is missing"))?;
        let fields = parse_multipart(&bytes, &boundary)?;
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        let scope = field("scope").and_then(|value| String::from_utf8(value).ok());
        let path = field("path")
            .and_then(|value| String::from_utf8(value).ok())
            .ok_or_else(|| ApiError::bad_request("a path is required"))?;
        let file = field("file")
            .or_else(|| field("content"))
            .ok_or_else(|| ApiError::bad_request("file content is required"))?;
        (scope, path, params_for_bytes(&file))
    } else if content_type.starts_with("application/json") {
        let body: WriteRequest = serde_json::from_slice(&bytes)
            .map_err(|error| ApiError::bad_request(format!("invalid upload body: {error}")))?;
        let params = params_for_json(&body)?;
        (body.scope, body.path, params)
    } else {
        let path = query
            .path
            .ok_or_else(|| ApiError::bad_request("a path is required"))?;
        (query.scope, path, params_for_bytes(&bytes))
    };

    let scope = scope.unwrap_or_else(|| "www".into());
    check_scope(&state, &user, &scope, Some(&path)).await?;
    let mut params = content;
    params.insert("scope".into(), json!(scope));
    params.insert("path".into(), json!(path));
    let result: Value = state
        .core_call(methods::FILES_WRITE, Value::Object(params))
        .await?;
    audit(&state, &user.user_id, "", "files.upload", &path, &user.ip).await;
    Ok(Json(result))
}

/// The parameters `FILES_WRITE` needs for a JSON upload body.
fn params_for_json(body: &WriteRequest) -> Result<Map<String, Value>, ApiError> {
    let mut params = Map::new();
    if let Some(content) = &body.content {
        params.insert("content".into(), json!(content));
    }
    if let Some(encoded) = &body.content_base64 {
        params.insert("content_base64".into(), json!(encoded));
    }
    if params.is_empty() {
        return Err(ApiError::bad_request("file content is required"));
    }
    Ok(params)
}

/// Binary-safe upload: base64 survives the IPC hop unchanged.
fn params_for_bytes(bytes: &[u8]) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert(
        "content_base64".into(),
        json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
    );
    params
}

fn multipart_boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if name.eq_ignore_ascii_case("boundary") {
            Some(value.trim().trim_matches('"').to_string())
        } else {
            None
        }
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Parse just enough of `multipart/form-data` for the upload endpoint.
/// Field bodies are kept as raw bytes so binary files are not altered.
fn parse_multipart(bytes: &[u8], boundary: &str) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let opening = format!("--{boundary}");
    let separator = format!("\r\n--{boundary}");
    let mut cursor = find_bytes(bytes, opening.as_bytes(), 0)
        .ok_or_else(|| ApiError::bad_request("invalid multipart body"))?
        + opening.len();
    let mut fields = Vec::new();
    loop {
        if bytes.get(cursor..cursor.saturating_add(2)) == Some(b"--") {
            break;
        }
        if bytes.get(cursor..cursor.saturating_add(2)) != Some(b"\r\n") {
            return Err(ApiError::bad_request("invalid multipart delimiter"));
        }
        cursor += 2;
        let headers_end = find_bytes(bytes, b"\r\n\r\n", cursor)
            .ok_or_else(|| ApiError::bad_request("invalid multipart part"))?;
        let headers = std::str::from_utf8(&bytes[cursor..headers_end])
            .map_err(|_| ApiError::bad_request("invalid multipart headers"))?;
        let name = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.eq_ignore_ascii_case("content-disposition") {
                    return None;
                }
                value.split(';').find_map(|part| {
                    let (key, value) = part.trim().split_once('=')?;
                    if key.eq_ignore_ascii_case("name") {
                        Some(value.trim().trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| ApiError::bad_request("multipart part has no name"))?;
        let content_start = headers_end + 4;
        let next = find_bytes(bytes, separator.as_bytes(), content_start)
            .ok_or_else(|| ApiError::bad_request("unterminated multipart part"))?;
        fields.push((name, bytes[content_start..next].to_vec()));
        cursor = next + separator.len();
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multipart_boundary() {
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=\"x-y\"").as_deref(),
            Some("x-y")
        );
        assert!(multipart_boundary("application/json").is_none());
    }

    #[test]
    fn parses_multipart_fields_without_changing_binary_bytes() {
        let body = b"--b\r\n\
Content-Disposition: form-data; name=\"scope\"\r\n\r\nwww\r\n\
--b\r\n\
Content-Disposition: form-data; name=\"path\"\r\n\r\nassets/a.bin\r\n\
--b\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\
Content-Type: application/octet-stream\r\n\r\n\x00\x01\r\n\xff\r\n--b--\r\n";
        let fields = parse_multipart(body, "b").unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].0, "scope");
        assert_eq!(fields[0].1, b"www");
        assert_eq!(fields[1].0, "path");
        assert_eq!(fields[1].1, b"assets/a.bin");
        assert_eq!(fields[2].0, "file");
        assert_eq!(fields[2].1, b"\x00\x01\r\n\xff");
    }
}
