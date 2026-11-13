#![allow(clippy::unwrap_used, dead_code)]

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use corvus_core::Corvus;
use corvus_core::hermod::{Hermod, HermodConfig};
use corvus_core::testing::TestDb;
use corvus_http::ApiToken;
use serde_json::{Value, json};
use tower::ServiceExt;

pub const TOKEN: &str = "test-token-0123456789-abcdefghijklmnopqrstuvwxyz";
pub const PORT: u16 = 47_001;

pub struct Env {
    pub t: TestDb,
    pub corvus: Corvus,
    pub hermod: Hermod,
    pub app: Router,
}

pub fn env() -> Env {
    let t = TestDb::new();
    // Fast watcher so long-poll tests run quickly.
    let hermod = Hermod::with_config(
        &t.path,
        HermodConfig {
            poll: Duration::from_millis(50),
            recheck: Duration::from_secs(60),
            detect: true,
        },
    );
    let corvus = Corvus::new(t.services(), hermod.clone());
    let app = corvus_http::app(corvus.clone(), ApiToken::new(TOKEN).unwrap(), PORT);
    Env {
        t,
        corvus,
        hermod,
        app,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("body is not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn code(&self) -> String {
        self.json()["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

/// A request builder: `Req::get("/api/x").token().json(..)`.
pub struct Req(pub axum::http::request::Builder, pub Vec<u8>);

impl Req {
    pub fn new(method: &str, path: &str) -> Self {
        Self(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", format!("127.0.0.1:{PORT}")),
            Vec::new(),
        )
    }

    /// No `Host` header at all.
    pub fn bare(method: &str, path: &str) -> Self {
        Self(Request::builder().method(method).uri(path), Vec::new())
    }

    pub fn host(self, h: &str) -> Self {
        self.header("host", h)
    }

    pub fn get(path: &str) -> Self {
        Self::new("GET", path)
    }

    pub fn post(path: &str) -> Self {
        Self::new("POST", path)
    }

    pub fn token(self) -> Self {
        self.header("authorization", &format!("Bearer {TOKEN}"))
    }

    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.0 = self.0.header(k, v);
        self
    }

    pub fn json(mut self, v: &Value) -> Self {
        self.0 = self.0.header("content-type", "application/json");
        self.1 = serde_json::to_vec(v).unwrap();
        self
    }

    pub fn raw(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.0 = self.0.header("content-type", content_type);
        self.1 = body;
        self
    }

    pub fn build(self) -> Request<Body> {
        self.0.body(Body::from(self.1)).unwrap()
    }
}

pub async fn send(app: &Router, req: Req) -> Reply {
    let res = app.clone().oneshot(req.build()).await.unwrap();
    let (parts, body) = res.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

pub async fn get(app: &Router, path: &str) -> Reply {
    send(app, Req::get(path).token()).await
}

pub async fn post(app: &Router, path: &str, body: &Value) -> Reply {
    send(app, Req::post(path).token().json(body)).await
}

/// Registers a session for `agent` and returns its id.
pub async fn register(app: &Router, agent: &str) -> String {
    let r = post(
        app,
        "/api/sessions",
        &json!({"agent_name": agent, "client_name": "test"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.body));
    r.json()["id"].as_str().unwrap().to_owned()
}
