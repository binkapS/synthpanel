//! API error type and helpers.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

impl From<crate::ipc::CoreError> for ApiError {
    fn from(error: crate::ipc::CoreError) -> Self {
        match error {
            crate::ipc::CoreError::Unavailable(message) => {
                ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "core_unavailable", message)
            }
            crate::ipc::CoreError::Remote { code, message } => {
                let status = match code.as_str() {
                    "not_found" => StatusCode::NOT_FOUND,
                    "conflict" => StatusCode::CONFLICT,
                    "bad_request" => StatusCode::BAD_REQUEST,
                    "unauthorized" => StatusCode::UNAUTHORIZED,
                    "rate_limited" => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::BAD_REQUEST,
                };
                ApiError::new(status, &code, message)
            }
            crate::ipc::CoreError::Decode(message) => ApiError::internal(message),
        }
    }
}
