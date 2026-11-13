//! The error model ([system/04](../../../spec/system/04-interfaces.md#error-model)): every failure leaves as
//! `{"error": {"code": "...", "message": "..."}}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use corvus_domain::CorvusError;
use serde_json::json;

/// An HTTP error: status plus the stable code and message. Build one from a [`CorvusError`] with `From`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// 403 for a rejected `Host` header.
    pub fn forbidden_host() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden_host", "host not allowed")
    }

    /// 401 for a missing or wrong bearer token.
    pub fn unauthorized() -> Self {
        Self::from(CorvusError::Unauthorized)
    }

    /// 413 for a body over the cap.
    pub fn payload_too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "request body exceeds 256 KiB",
        )
    }
}

impl From<CorvusError> for ApiError {
    fn from(e: CorvusError) -> Self {
        let status = match &e {
            CorvusError::NotFound(_) => StatusCode::NOT_FOUND,
            CorvusError::Conflict(_) | CorvusError::SessionNotActive => StatusCode::CONFLICT,
            CorvusError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            CorvusError::Unauthorized => StatusCode::UNAUTHORIZED,
            CorvusError::Busy => StatusCode::SERVICE_UNAVAILABLE,
            CorvusError::Internal(detail) => {
                // The detail stays in the log; the client gets a generic message.
                tracing::error!(detail = %detail, "internal error");
                return Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error");
            }
        };
        Self::new(status, e.code(), e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": {"code": self.code, "message": self.message}})),
        )
            .into_response()
    }
}
