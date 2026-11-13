//! The HTTP API ([system/04](../../../spec/system/04-interfaces.md#http-api), [system/08](../../../spec/system/08-security.md)).
//!
//! Two things: [`data_router`], the inner router (handlers only, no auth; never bound to a socket — the desktop
//! app calls it in memory), and [`app`] / [`serve`], the outer application (host allow-list → bearer auth →
//! body limit → timeout → logging) which is the only thing ever bound. No pages, no docs, no schema routes.

mod error;
mod extract;
mod middleware;
mod routes;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use corvus_core::Corvus;
use corvus_domain::CorvusError;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use error::ApiError;
pub use routes::{data_router, route_table};

/// Request body cap (system/11 row 19).
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Whole-request timeout: the 120 s long poll plus margin.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(130);

/// How long shutdown waits for in-flight requests (a long poll can last two minutes) before giving up.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

const MIN_TOKEN_CHARS: usize = 32;

/// The bearer token, held only as a SHA-256 digest. Deliberately no `PartialEq`: see [`ApiToken::verify`].
pub struct ApiToken {
    digest: [u8; 32],
}

impl std::fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

impl ApiToken {
    /// Rejects tokens shorter than 32 characters.
    pub fn new(token: &str) -> Result<Self, CorvusError> {
        if token.chars().count() < MIN_TOKEN_CHARS {
            return Err(CorvusError::Validation(format!(
                "the API token must be at least {MIN_TOKEN_CHARS} characters"
            )));
        }
        Ok(Self {
            digest: Sha256::digest(token.as_bytes()).into(),
        })
    }

    /// Constant-time comparison of the SHA-256 digests of the presented and the stored token.
    pub fn verify(&self, presented: &str) -> bool {
        let d: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
        self.digest[..].ct_eq(&d[..]).into()
    }
}

/// What the outer layers need: the token and the configured port (for the `Host` allow-list).
#[derive(Debug)]
pub(crate) struct Guard {
    pub token: ApiToken,
    pub port: u16,
}

/// The secured application: `data_router` wrapped in host allow-list → bearer auth → body limit → timeout,
/// with request logging and the security headers outside all of them. This is the only thing ever bound.
pub fn app(corvus: Corvus, token: ApiToken, port: u16) -> Router {
    let guard = Arc::new(Guard { token, port });
    data_router(corvus)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(from_fn(middleware::body_limit))
        .layer(from_fn_with_state(guard.clone(), middleware::auth))
        .layer(from_fn_with_state(guard, middleware::host))
        .layer(from_fn(middleware::request_log))
        .layer(from_fn(middleware::security_headers))
}

/// Binds `127.0.0.1:port` (never configurable) and serves the secured API until `shutdown` resolves.
pub async fn serve(
    corvus: Corvus,
    token: ApiToken,
    port: u16,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let router = app(corvus, token, port);
    let (stopping_tx, stopping_rx) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        shutdown.await;
        let _ = stopping_tx.send(());
    });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        r = &mut server => r,
        () = async {
            // Only start the grace clock once shutdown was requested.
            if stopping_rx.await.is_ok() {
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => Ok(()),
    }
}
