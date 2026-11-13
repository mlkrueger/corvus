//! Request extractors whose rejections use the error model (422 with the JSON shape, never axum's plain text).

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, JsonRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use corvus_domain::CorvusError;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

fn validation(msg: impl Into<String>) -> ApiError {
    CorvusError::Validation(msg.into()).into()
}

/// A JSON request body.
#[derive(Debug)]
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(Self(v)),
            Err(JsonRejection::BytesRejection(r)) if r.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::payload_too_large())
            }
            Err(e) => Err(validation(e.body_text())),
        }
    }
}

/// A JSON request body that may be empty (then `T::default()`), for routes whose body is all-optional.
#[derive(Debug)]
pub struct OptBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Default> FromRequest<S> for OptBody<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|r: BytesRejection| {
                if r.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::payload_too_large()
                } else {
                    validation("unreadable request body")
                }
            })?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self(T::default()));
        }
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|e| validation(format!("invalid JSON body: {e}")))
    }
}

/// Query-string parameters.
#[derive(Debug)]
pub struct Q<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for Q<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(v)| Self(v))
            .map_err(|e| validation(e.body_text()))
    }
}

/// One path parameter, as text (validated by the handler into its typed form).
#[derive(Debug)]
pub struct P(pub String);

impl<S: Send + Sync> FromRequestParts<S> for P {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Path::<String>::from_request_parts(parts, state)
            .await
            .map(|Path(v)| Self(v))
            .map_err(|_| validation("invalid path parameter"))
    }
}
