//! The `api` command's engine: build a request in memory, call the inner router, return status and JSON body.
//! No socket is involved.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, Uri, header};
use serde::Serialize;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Request body cap, as on the HTTP transport.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// Response cap (an export can be large; this only guards against runaway output).
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiResponse {
    pub status: u16,
    /// `null` when the response body is empty.
    pub body: Value,
}

/// Validates the request line. Only `GET`, `POST` and `PUT` on a relative `/api/...` path are allowed.
fn validate(method: &str, path: &str) -> Result<(Method, Uri), String> {
    let method = match method {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        other => return Err(format!("method not allowed: {other:?}")),
    };
    if !path.starts_with("/api/") {
        return Err("path must start with /api/".into());
    }
    if path
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\' || c == '#')
    {
        return Err("path contains forbidden characters".into());
    }
    let route = path.split('?').next().unwrap_or(path);
    if route.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err("path must not contain dot segments".into());
    }
    let uri: Uri = path.parse().map_err(|e| format!("invalid path: {e}"))?;
    if uri.scheme().is_some() || uri.authority().is_some() {
        return Err("path must not carry a scheme or host".into());
    }
    Ok((method, uri))
}

fn error_response(status: StatusCode, code: &str, message: &str) -> ApiResponse {
    ApiResponse {
        status: status.as_u16(),
        body: json!({ "error": { "code": code, "message": message } }),
    }
}

/// Runs one request against `router` (cloned; the original is untouched). HTTP-level errors are ordinary
/// responses; `Err` means the request was refused before reaching the router or could not be run.
pub async fn dispatch(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<ApiResponse, String> {
    let (method, uri) = validate(method, path)?;
    let mut req = Request::builder().method(method).uri(uri);
    let payload = match body {
        None | Some(Value::Null) => Body::empty(),
        Some(v) => {
            let bytes = serde_json::to_vec(&v).map_err(|e| format!("cannot encode body: {e}"))?;
            if bytes.len() > MAX_BODY_BYTES {
                return Ok(error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    "request body exceeds 256 KiB",
                ));
            }
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(bytes)
        }
    };
    let req = req
        .body(payload)
        .map_err(|e| format!("cannot build request: {e}"))?;
    let res = router
        .clone()
        .oneshot(req)
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = res.status().as_u16();
    let bytes = to_bytes(res.into_body(), MAX_RESPONSE_BYTES)
        .await
        .map_err(|e| format!("cannot read response: {e}"))?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok(ApiResponse { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rules() {
        assert!(validate("GET", "/api/health").is_ok());
        assert!(validate("PUT", "/api/x?a=b").is_ok());
        assert!(validate("DELETE", "/api/health").is_err());
        assert!(validate("get", "/api/health").is_err());
        assert!(validate("GET", "/health").is_err());
        assert!(validate("GET", "/api").is_err());
        assert!(validate("GET", "http://evil.example/api/health").is_err());
        assert!(validate("GET", "//evil.example/api/health").is_err());
        assert!(validate("GET", "/api/../etc").is_err());
        assert!(validate("GET", "/api/a b").is_err());
        assert!(validate("GET", "/api/a#frag").is_err());
    }
}
