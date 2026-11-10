//! Authentication endpoints: login, logout, session info and 2FA.

use crate::error::ApiError;
use crate::extract::{ApiJson, AuthUser, CSRF_COOKIE, SESSION_COOKIE};
use std::sync::Arc;

use crate::state::AppState;
use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use synthpanel_ipc::methods;

const LOGIN_WINDOW: Duration = Duration::from_secs(300);
const LOGIN_MAX_ATTEMPTS: usize = 10;
const CHALLENGE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct TwoFactorRequest {
    pub challenge_id: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct PasswordRequest {
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let ip = peer.ip().to_string();
    if !state
        .rate_limiter
        .check(&format!("login:{ip}"), LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW)
    {
        return Err(ApiError::too_many_requests(
            "too many login attempts; please try again later",
        ));
    }

    let email = request.email.trim().to_lowercase();
    let user: Option<Value> = state
        .core_call(methods::USERS_GET_BY_EMAIL, json!({ "email": email }))
        .await?;

    let Some(user) = user else {
        // Spend roughly the same time as a real verification to avoid
        // leaking whether the account exists.
        let _ = crate::auth::verify_password(
            &request.password,
            "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$0000000000000000000000000000000000000000000",
        );
        return Err(ApiError::unauthorized("invalid email or password"));
    };

    let hash = user
        .get("password_hash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !crate::auth::verify_password(&request.password, hash) {
        return Err(ApiError::unauthorized("invalid email or password"));
    }
    if user.get("status").and_then(Value::as_str) != Some("active") {
        return Err(ApiError::forbidden("this account is disabled"));
    }

    if user
        .get("totp_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let user_id = user
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let challenge_id = state.pending_2fa.create(user_id, CHALLENGE_TTL);
        return Ok(Json(json!({
            "requires_2fa": true,
            "challenge_id": challenge_id,
        }))
        .into_response());
    }

    establish_session(&state, &user, &ip, None).await
}

pub async fn totp_verify(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    ApiJson(request): ApiJson<TwoFactorRequest>,
) -> Result<Response, ApiError> {
    let ip = peer.ip().to_string();
    if !state
        .rate_limiter
        .check(&format!("login:{ip}"), LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW)
    {
        return Err(ApiError::too_many_requests(
            "too many login attempts; please try again later",
        ));
    }

    let Some(challenge) = state.pending_2fa.take(&request.challenge_id) else {
        return Err(ApiError::bad_request("invalid or expired challenge"));
    };

    let user: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": challenge.user_id }))
        .await?;
    let Some(user) = user else {
        return Err(ApiError::unauthorized("invalid email or password"));
    };

    let secret = user
        .get("totp_secret")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let email = user
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if secret.is_empty() || !crate::totp::verify(secret, &request.code, &email) {
        return Err(ApiError::unauthorized("invalid verification code"));
    }

    establish_session(&state, &user, &ip, None).await
}

pub async fn logout(State(state): State<Arc<AppState>>, user: AuthUser) -> Response {
    state.sessions.destroy(&user.token);
    let mut response = Json(json!({ "ok": true })).into_response();
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        clear_cookie(SESSION_COOKIE, state.config.secure_cookies),
    );
    headers.append(
        header::SET_COOKIE,
        clear_cookie(CSRF_COOKIE, state.config.secure_cookies),
    );
    response
}

pub async fn me(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let value: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": user.user_id }))
        .await?;
    match value {
        Some(value) => Ok(Json(sanitize_user(&value))),
        None => Err(ApiError::not_found("user no longer exists")),
    }
}

/// Generate a TOTP secret and store it (still disabled) for the account.
pub async fn totp_setup(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let record: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": user.user_id }))
        .await?;
    let record = record.ok_or_else(|| ApiError::not_found("user no longer exists"))?;
    let email = record
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let secret = crate::totp::generate_secret();
    let uri = crate::totp::provisioning_uri(&secret, &email)
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let _: Value = state
        .core_call(
            methods::USERS_UPDATE,
            json!({ "id": user.user_id, "totp_secret": secret, "totp_enabled": false }),
        )
        .await?;

    Ok(Json(json!({
        "secret": secret,
        "otpauth_url": uri,
    })))
}

/// Confirm the TOTP code and switch two-factor authentication on.
pub async fn totp_enable(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(request): ApiJson<CodeRequest>,
) -> Result<Json<Value>, ApiError> {
    let record: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": user.user_id }))
        .await?;
    let record = record.ok_or_else(|| ApiError::not_found("user no longer exists"))?;
    let secret = record
        .get("totp_secret")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let email = record
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if secret.is_empty() {
        return Err(ApiError::bad_request("start the 2FA setup first"));
    }
    if !crate::totp::verify(secret, &request.code, &email) {
        return Err(ApiError::bad_request("invalid verification code"));
    }

    let _: Value = state
        .core_call(
            methods::USERS_UPDATE,
            json!({ "id": user.user_id, "totp_enabled": true }),
        )
        .await?;
    audit(
        &state,
        &user.user_id,
        &email,
        "auth.2fa_enabled",
        &email,
        &user.ip,
    )
    .await;

    Ok(Json(json!({ "ok": true, "totp_enabled": true })))
}

/// Turn two-factor authentication off (password confirmation required).
pub async fn totp_disable(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    ApiJson(request): ApiJson<PasswordRequest>,
) -> Result<Json<Value>, ApiError> {
    let record: Option<Value> = state
        .core_call(methods::USERS_GET, json!({ "id": user.user_id }))
        .await?;
    let record = record.ok_or_else(|| ApiError::not_found("user no longer exists"))?;
    let hash = record
        .get("password_hash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !crate::auth::verify_password(&request.password, hash) {
        return Err(ApiError::unauthorized("invalid password"));
    }

    let _: Value = state
        .core_call(
            methods::USERS_UPDATE,
            json!({ "id": user.user_id, "totp_secret": null, "totp_enabled": false }),
        )
        .await?;
    audit(&state, &user.user_id, "", "auth.2fa_disabled", "", &user.ip).await;

    Ok(Json(json!({ "ok": true, "totp_enabled": false })))
}

/// Strip secrets before sending a user record to the browser.
pub fn sanitize_user(user: &Value) -> Value {
    let mut out = user.clone();
    if let Some(object) = out.as_object_mut() {
        object.remove("password_hash");
        object.remove("totp_secret");
    }
    out
}

async fn establish_session(
    state: &AppState,
    user: &Value,
    ip: &str,
    user_agent: Option<&str>,
) -> Result<Response, ApiError> {
    let user_id = user
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let email = user
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = user
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let csrf = crate::auth::generate_csrf();
    let ttl = Duration::from_secs(state.config.session_ttl_secs);
    let token = state.sessions.create(
        user_id.clone(),
        ip.to_string(),
        user_agent.map(|value| value.to_string()),
        ttl,
        csrf.clone(),
    );

    let _: Value = state
        .core_call(
            methods::USERS_UPDATE,
            json!({ "id": user_id, "last_login_at": Utc::now().to_rfc3339() }),
        )
        .await?;
    audit(state, &user_id, &name, "auth.login", &email, ip).await;

    let mut response = Json(json!({
        "requires_2fa": false,
        "user": sanitize_user(user),
    }))
    .into_response();

    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        session_cookie(
            SESSION_COOKIE,
            &token,
            ttl.as_secs() as i64,
            state.config.secure_cookies,
            true,
        ),
    );
    headers.append(
        header::SET_COOKIE,
        session_cookie(
            CSRF_COOKIE,
            &csrf,
            ttl.as_secs() as i64,
            state.config.secure_cookies,
            false,
        ),
    );
    Ok(response)
}

/// Record an audit entry; audit failures never break the request.
pub async fn audit(
    state: &AppState,
    actor_id: &str,
    actor_name: &str,
    action: &str,
    target: &str,
    ip: &str,
) {
    let result: Result<Value, _> = state
        .core_call(
            methods::AUDIT_RECORD,
            json!({
                "actor_id": actor_id,
                "actor_name": actor_name,
                "action": action,
                "target": target,
                "ip": ip,
                "metadata": {},
            }),
        )
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, action, "failed to write audit entry");
    }
}

fn session_cookie(
    name: &str,
    value: &str,
    max_age: i64,
    secure: bool,
    http_only: bool,
) -> HeaderValue {
    let mut cookie = format!("{name}={value}; Path=/; Max-Age={max_age}; SameSite=Strict");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("cookie header is valid")
}

fn clear_cookie(name: &str, secure: bool) -> HeaderValue {
    let mut cookie = format!("{name}=; Path=/; Max-Age=0; SameSite=Strict");
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("cookie header is valid")
}
