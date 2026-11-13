//! The outer layers: security headers, request logging, host allow-list, bearer auth, body-size check.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, HOST, HeaderName, HeaderValue, WWW_AUTHENTICATE,
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::{Guard, MAX_BODY_BYTES};

/// `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every response, errors and 404s included.
pub(crate) async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    res
}

/// One line per request: method, matched route template, status, duration. Never the URI (query strings),
/// headers or bodies.
pub(crate) async fn request_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "<unmatched>".to_owned(), |m| m.as_str().to_owned());
    let started = Instant::now();
    let res = next.run(req).await;
    tracing::info!(
        method = %method,
        route = %route,
        status = res.status().as_u16(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "request"
    );
    res
}

/// `Host` must be `127.0.0.1`, `localhost` or `[::1]`, with the configured port or none. Checked before auth.
pub(crate) async fn host(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let mut hosts = req.headers().get_all(HOST).iter();
    let allowed = match (hosts.next(), hosts.next()) {
        (Some(v), None) => v.to_str().is_ok_and(|v| host_allowed(v, g.port)),
        _ => false,
    };
    if allowed {
        next.run(req).await
    } else {
        ApiError::forbidden_host().into_response()
    }
}

fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    let (name, suffix) = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some(("::1", after)) => ("[::1]", after),
            _ => return false,
        }
    } else {
        match host.find(':') {
            Some(i) => (&host[..i], &host[i..]),
            None => (host.as_str(), ""),
        }
    };
    if !matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        return false;
    }
    match suffix {
        "" => true,
        s => s.strip_prefix(':').is_some_and(|p| p == port.to_string()),
    }
}

/// `Authorization: Bearer <token>` on every request; no route is exempt.
pub(crate) async fn auth(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let presented = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let (scheme, rest) = v.split_at_checked(7)?;
            scheme.eq_ignore_ascii_case("bearer ").then_some(rest)
        });
    if presented.is_some_and(|t| g.token.verify(t)) {
        next.run(req).await
    } else {
        let mut res = ApiError::unauthorized().into_response();
        res.headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        res
    }
}

/// Rejects a declared `Content-Length` over the cap with the JSON error shape; the body-limit layer behind it
/// enforces the cap on chunked bodies.
pub(crate) async fn body_limit(req: Request, next: Next) -> Response {
    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|n| n > MAX_BODY_BYTES) {
        return ApiError::payload_too_large().into_response();
    }
    next.run(req).await
}
