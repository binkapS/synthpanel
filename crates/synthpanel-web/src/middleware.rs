//! Security middleware: response headers, gatekeeping and CSRF protection.

use crate::error::ApiError;
use crate::extract::{CSRF_COOKIE, CSRF_HEADER, SESSION_COOKIE, cookie_value};
use std::sync::Arc;

use crate::config::PanelBasicAuth;
use crate::state::AppState;
use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use subtle::ConstantTimeEq;
use synthpanel_ipc::methods;

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     base-uri 'self'; \
     object-src 'none'; \
     frame-ancestors 'none'; \
     form-action 'self'; \
     img-src 'self' data: blob:; \
     font-src 'self' data:; \
     style-src 'self' 'unsafe-inline'; \
     script-src 'self' 'wasm-unsafe-eval'; \
     connect-src 'self' ws: wss:";

/// Endpoints that are reachable without a session (and therefore without a
/// CSRF token).
const CSRF_EXEMPT: &[&str] = &["/api/v1/auth/login", "/api/v1/auth/2fa/verify"];

/// Public endpoints that stay reachable while the panel gate is active: the
/// load balancer probe and the ACME HTTP-01 challenge must never be hidden
/// behind the extra credentials.
const ACME_PREFIX: &str = "/.well-known/acme-challenge/";

/// Requests that maintenance mode never blocks. Login stays open so an admin
/// can still sign in and turn maintenance mode off; logout is harmless.
const MAINTENANCE_EXEMPT: &[&str] = &[
    "/healthz",
    "/api/v1/auth/login",
    "/api/v1/auth/2fa/verify",
    "/api/v1/auth/logout",
];

const MAINTENANCE_HTML: &str = "<!doctype html>\
    <html lang=\"en\"><head><meta charset=\"utf-8\">\
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
    <title>Maintenance — SynthPanel</title></head>\
    <body><h1>Maintenance in progress</h1>\
    <p>The control panel is temporarily unavailable. Please try again shortly.</p>\
    </body></html>";

/// Add hardening headers to every response.
pub async fn security_headers(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let secure =
        state.config.secure_cookies && crate::api::auth::request_is_secure(request.headers());
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
    // HSTS only makes sense when the browser itself talks TLS. Behind a
    // proxy that terminates TLS the browser may be on plain HTTP, and the
    // header would lock it into HTTPS.
    if secure {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}

/// The address of the client for the panel gate.
///
/// As in the authorization middleware, forwarded headers are only honoured
/// when the panel is configured to sit behind a trusted proxy. The flag comes
/// from the application state rather than from a second read of the process
/// environment, so the gate and the audit log agree.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trusted_proxies: bool) -> String {
    if trusted_proxies {
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

/// Verify an HTTP Basic header against the configured Argon2id hash.
///
/// The username is compared in constant time and the password is always
/// verified, so a wrong username does not take a measurably different path.
pub fn basic_auth_valid(headers: &HeaderMap, config: &PanelBasicAuth) -> bool {
    if !config.is_complete() {
        return false;
    }
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some((scheme, encoded)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Ok(pair) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some((user, password)) = pair.split_once(':') else {
        return false;
    };
    let user_matches: bool = user.as_bytes().ct_eq(config.user.as_bytes()).into();
    let password_matches = crate::auth::verify_password(password, &config.password_hash);
    user_matches && password_matches
}

/// Interpret a stored setting that acts as a boolean.
///
/// Settings are free-form JSON: the UI, CLI and IPC callers have written
/// booleans, numbers and strings over time. Only positive spellings enable
/// the flag; everything else (including a missing key) leaves it off.
pub fn setting_flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(text)) => matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on" | "enabled"
        ),
        _ => false,
    }
}

/// Whether maintenance mode affects a request.
///
/// Public probes, ACME challenges and the login flow stay reachable, and an
/// administrator may keep working on the panel to switch maintenance off.
pub fn maintenance_applies(path: &str, role: Option<&str>) -> bool {
    if MAINTENANCE_EXEMPT.contains(&path) || path.starts_with(ACME_PREFIX) {
        return false;
    }
    role != Some("admin")
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

fn basic_challenge() -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "Authentication required.\n").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"SynthPanel\", charset=\"UTF-8\""),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn maintenance_response(path: &str) -> Response {
    let mut response = if path.starts_with("/api/") || path.starts_with("/ws/") {
        json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "maintenance",
            "the panel is in maintenance mode",
        )
    } else {
        let mut response = (StatusCode::SERVICE_UNAVAILABLE, MAINTENANCE_HTML).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        response
    };
    let headers = response.headers_mut();
    headers.insert(header::RETRY_AFTER, HeaderValue::from_static("300"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The panel-level gate: IP allow-list, optional HTTP Basic authentication
/// and enforcement of the stored maintenance mode.
///
/// This runs before session authorization and CSRF: neither an unauthenticated
/// browser nor an API key gets to probe the panel while a gate is closed. The
/// gate itself never consumes the request body.
pub async fn panel_gate(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if path == "/healthz" || path.starts_with(ACME_PREFIX) {
        return next.run(request).await;
    }

    // Copy the headers and peer before any await: the request body is not
    // `Sync`, so the whole request may not be held across one.
    let headers = request.headers().clone();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0);

    if state.config.ip_allow_list_enabled() {
        let source = client_ip(&headers, peer, state.config.trusts_proxies());
        let allowed = source
            .parse::<IpAddr>()
            .map(|ip| state.config.allows_ip(&ip))
            .unwrap_or(false);
        if !allowed {
            tracing::warn!(ip = %source, path = %path, "panel access denied by IP allow-list");
            return json_error(StatusCode::FORBIDDEN, "access_denied", "access denied");
        }
    }

    if let Some(basic) = &state.config.panel_basic_auth {
        if !basic_auth_valid(&headers, basic) {
            if !basic.is_complete() {
                tracing::error!(
                    "panel Basic auth is enabled but the username or Argon2id hash is missing or invalid"
                );
            }
            return basic_challenge();
        }
    }

    let setting: Value = match state
        .core_call(methods::SETTINGS_GET, json!({ "key": "maintenance_mode" }))
        .await
    {
        Ok(value) => value,
        Err(error) => {
            // An unavailable core must not lock everybody out of login; the
            // next request reads the setting again.
            tracing::warn!(error = %error, "cannot read maintenance_mode; treating it as off");
            Value::Null
        }
    };
    if setting_flag(Some(&setting)) {
        let role = match crate::authz::resolve_actor(&state, &headers, peer).await {
            Ok(Some(context)) => Some(context.role),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(error = ?error, "cannot resolve actor during maintenance");
                None
            }
        };
        if maintenance_applies(&path, role.as_deref()) {
            return maintenance_response(&path);
        }
    }

    next.run(request).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IpRange;
    use std::net::Ipv4Addr;

    #[test]
    fn maintenance_blocks_pages_and_apis_but_not_public_paths() {
        for path in [
            "/",
            "/dashboard",
            "/api/v1/sites",
            "/api/v1/auth/me",
            "/ws/logs",
        ] {
            assert!(maintenance_applies(path, None), "{path}");
            assert!(maintenance_applies(path, Some("user")), "{path}");
            assert!(maintenance_applies(path, Some("operator")), "{path}");
        }
        for path in [
            "/healthz",
            "/.well-known/acme-challenge/token",
            "/api/v1/auth/login",
            "/api/v1/auth/2fa/verify",
            "/api/v1/auth/logout",
        ] {
            assert!(!maintenance_applies(path, None), "{path}");
            assert!(!maintenance_applies(path, Some("user")), "{path}");
        }
        assert!(!maintenance_applies("/api/v1/settings", Some("admin")));
        assert!(maintenance_applies("/api/v1/settings", Some("guest")));
    }

    #[test]
    fn maintenance_reads_every_setting_shape() {
        assert!(setting_flag(Some(&json!(true))));
        assert!(!setting_flag(Some(&json!(false))));
        assert!(setting_flag(Some(&json!(1))));
        assert!(!setting_flag(Some(&json!(0))));
        for value in ["true", "TRUE", " 1 ", "yes", "on", "enabled"] {
            assert!(setting_flag(Some(&json!(value))), "{value}");
        }
        for value in ["false", "0", "no", "off", "", "maybe"] {
            assert!(!setting_flag(Some(&json!(value))), "{value}");
        }
        assert!(!setting_flag(None));
        assert!(!setting_flag(Some(&Value::Null)));
        assert!(!setting_flag(Some(&json!([]))));
    }

    #[test]
    fn forwarded_for_is_only_trusted_when_configured() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9, 10.0.0.1".parse().unwrap());
        let peer = Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            1234,
        ));
        assert_eq!(client_ip(&headers, peer, false), "192.0.2.10");
        assert_eq!(client_ip(&headers, peer, true), "203.0.113.9");
        headers.insert("x-forwarded-for", " , 203.0.113.9".parse().unwrap());
        assert_eq!(client_ip(&headers, peer, true), "192.0.2.10");
    }

    #[test]
    fn basic_auth_requires_both_parts_and_the_right_scheme() {
        use crate::auth::hash_password;
        let hash = hash_password("correct horse battery staple").unwrap();
        let config = PanelBasicAuth {
            user: "operator".into(),
            password_hash: hash.clone(),
        };
        let encoded = base64::engine::general_purpose::STANDARD
            .encode("operator:correct horse battery staple");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {encoded}").parse().unwrap(),
        );
        assert!(basic_auth_valid(&headers, &config));
        headers.insert(
            header::AUTHORIZATION,
            format!("basic {encoded}").parse().unwrap(),
        );
        assert!(basic_auth_valid(&headers, &config));

        for value in [
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("operator:wrong")
            ),
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD
                    .encode("other:correct horse battery staple")
            ),
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("no-colon")
            ),
            "Bearer something".into(),
            "Basic !!!".into(),
        ] {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            assert!(!basic_auth_valid(&headers, &config), "{value}");
        }
        headers.clear();
        assert!(!basic_auth_valid(&headers, &config));

        let incomplete = PanelBasicAuth {
            user: "".into(),
            password_hash: hash,
        };
        let valid = base64::engine::general_purpose::STANDARD
            .encode("operator:correct horse battery staple");
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {valid}").parse().unwrap(),
        );
        assert!(!basic_auth_valid(&headers, &incomplete));
    }

    #[test]
    fn gate_rejections_have_the_right_shape() {
        let api = maintenance_response("/api/v1/sites");
        assert_eq!(api.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            api.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert!(api.headers().contains_key(header::RETRY_AFTER));

        let page = maintenance_response("/dashboard");
        assert_eq!(page.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            page.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let challenge = basic_challenge();
        assert_eq!(challenge.status(), StatusCode::UNAUTHORIZED);
        assert!(
            challenge
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("Basic realm=")
        );
    }

    #[test]
    fn an_ip_allow_list_is_matched_after_the_gate_resolves_the_client() {
        let ranges = crate::config::parse_ip_list("192.0.2.0/24");
        let headers = HeaderMap::new();
        let peer = Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            1234,
        ));
        let source = client_ip(&headers, peer, false);
        let ip: IpAddr = source.parse().unwrap();
        assert!(crate::config::ip_allowed(&ranges, &ip));
        let ranges = [IpRange::parse("198.51.100.0/24").unwrap()];
        assert!(!crate::config::ip_allowed(&ranges, &ip));
    }
}
