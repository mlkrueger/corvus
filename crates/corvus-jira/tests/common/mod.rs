#![allow(dead_code, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, Method, Uri};
use corvus_domain::SourceFilter;
use corvus_jira::JiraSource;
use serde_json::Value;

pub const TOKEN: &str = "s3cr3t-token-XYZ";
pub const EMAIL: &str = "ada@example.test";

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub body: String,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
    pub delay: Duration,
}

impl Resp {
    pub fn json(v: Value) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "application/json".into())],
            body: v.to_string(),
            delay: Duration::ZERO,
        }
    }
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: vec![],
            body: "{}".into(),
            delay: Duration::ZERO,
        }
    }
}

pub type Log = Arc<Mutex<Vec<Req>>>;

pub async fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> (String, Log) {
    let log: Log = Arc::default();
    let handler = Arc::new(handler);
    let l = log.clone();
    let app = Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: String| {
            let (handler, l) = (handler.clone(), l.clone());
            async move {
                let req = Req {
                    method: method.to_string(),
                    path: uri.path_and_query().map(ToString::to_string).unwrap_or_default(),
                    auth: headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned),
                    body,
                };
                l.lock().unwrap().push(req.clone());
                let r = handler(&req);
                tokio::time::sleep(r.delay).await;
                let mut b = axum::http::Response::builder().status(r.status);
                for (k, v) in r.headers {
                    b = b.header(k, v);
                }
                b.body(axum::body::Body::from(r.body)).unwrap()
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, log)
}

pub fn source(base: &str) -> JiraSource {
    source_with(base, Duration::from_secs(5), Duration::from_secs(10), 1 << 20)
}

pub fn source_with(base: &str, connect: Duration, total: Duration, cap: usize) -> JiraSource {
    JiraSource::for_tests(base, EMAIL, TOKEN, connect, total, cap).unwrap()
}

pub fn filter() -> SourceFilter {
    corvus_domain::jql::default_filter()
}
