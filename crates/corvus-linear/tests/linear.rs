//! The Linear adapter against a real loopback fake GraphQL server (no real Linear workspace is used).
#![allow(clippy::unwrap_used)]

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};
use corvus_core::Config;
use corvus_core::config::LinearConfig;
use corvus_core::sources::WorkSource;
use corvus_domain::linear::{COMMENTS_QUERY, SEARCH_QUERY, STATUSES_QUERY};
use corvus_domain::{SourceFilter, SourceId, StatusCategory, TicketKey};
use corvus_linear::{Limits, LinearSource, from_config};
use serde_json::{Value, json};

const KEY: &str = "lin_api_SECRETKEY1234567890";
const NOW: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

enum Reply {
    Json(Value),
    Status(u16, &'static str),
    Redirect,
    Sleep(Duration),
    Raw(Vec<u8>),
}

#[derive(Default)]
struct Fake {
    script: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<(Option<String>, Value)>>,
    other_hits: Mutex<u32>,
}

async fn graphql(State(f): State<Arc<Fake>>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = headers
        .get("authorization")
        .map(|v| v.to_str().unwrap().to_owned());
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    f.requests.lock().unwrap().push((auth, v));
    let reply = f.script.lock().unwrap().pop_front().expect("unscripted request");
    match reply {
        Reply::Json(v) => axum::Json(v).into_response(),
        Reply::Status(c, b) => (StatusCode::from_u16(c).unwrap(), b).into_response(),
        Reply::Redirect => (StatusCode::FOUND, [(header::LOCATION, "/other")]).into_response(),
        Reply::Sleep(d) => {
            tokio::time::sleep(d).await;
            axum::Json(json!({"data": {}})).into_response()
        }
        Reply::Raw(b) => b.into_response(),
    }
}

async fn other(State(f): State<Arc<Fake>>) -> &'static str {
    *f.other_hits.lock().unwrap() += 1;
    "{}"
}

async fn start(script: Vec<Reply>) -> (Arc<Fake>, String) {
    let fake = Arc::new(Fake {
        script: Mutex::new(script.into()),
        ..Fake::default()
    });
    let app = Router::new()
        .route("/graphql", post(graphql))
        .route("/other", any(other))
        .with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (fake, url)
}

fn cfg() -> LinearConfig {
    let m: HashMap<&str, &str> = HashMap::from([
        ("HOME", "/home/u"),
        ("CORVUS_LINEAR_API_KEY", KEY),
        ("CORVUS_LINEAR_POLL_SECONDS", "42"),
        ("CORVUS_LINEAR_POOL", "lin"),
        ("CORVUS_LINEAR_IGNORE_USERS", "u1, u2"),
    ]);
    Config::from_lookup(|k| m.get(k).map(|s| (*s).to_owned()))
        .unwrap()
        .linear
        .unwrap()
}

async fn source(script: Vec<Reply>) -> (Arc<Fake>, LinearSource) {
    source_with(script, Limits::default()).await
}

async fn source_with(script: Vec<Reply>, limits: Limits) -> (Arc<Fake>, LinearSource) {
    log_buffer();
    let (f, url) = start(script).await;
    (
        f,
        LinearSource::with_endpoint_for_tests(&url, &cfg(), limits).unwrap(),
    )
}

fn issue(id: &str) -> Value {
    json!({
        "identifier": id, "title": format!("title {id}"), "description": "desc **md**",
        "url": format!("https://linear.app/acme/issue/{id}/x"), "priority": 2,
        "updatedAt": "2026-01-01T10:00:00.123Z", "completedAt": null,
        "state": {"name": "In Progress", "type": "started"}, "team": {"key": "ENG"},
        "assignee": {"name": "Ada"}
    })
}

fn page(nodes: Vec<Value>, next: Option<&str>) -> Reply {
    Reply::Json(json!({"data": {"issues": {"nodes": nodes,
        "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}}}}))
}

fn filter() -> SourceFilter {
    SourceFilter {
        statuses: vec!["Todo".into(), "Won't \"fix\" \\ {x}".into()],
        done_days: 7,
        projects: vec!["ENG".into()],
        auto_queue: vec![],
    }
}

#[tokio::test]
async fn search_request_shape_and_variables_only() {
    let (f, s) = source(vec![page(vec![issue("ENG-1")], None)]).await;
    let ts = s.search(&filter(), NOW).await.unwrap();
    assert_eq!(ts.len(), 1);
    let reqs = f.requests.lock().unwrap();
    let (auth, body) = &reqs[0];
    assert_eq!(auth.as_deref(), Some(KEY), "raw key, no Bearer");
    assert_eq!(body["query"], SEARCH_QUERY);
    let qtext = body["query"].as_str().unwrap();
    assert!(!qtext.contains("fix") && !qtext.contains("ENG") && !qtext.contains("2025-12-25"));
    assert!(!qtext.contains("mutation"));
    assert_eq!(
        body["variables"],
        json!({"cursor": null, "filter": {
            "assignee": {"isMe": {"eq": true}},
            "or": [
                {"state": {"name": {"in": ["Todo", "Won't \"fix\" \\ {x}"]}}},
                {"completedAt": {"gte": "2025-12-25T00:00:00.000Z"}}
            ],
            "team": {"key": {"in": ["ENG"]}}
        }})
    );
}

#[tokio::test]
async fn control_chars_rejected_before_any_request() {
    let (f, s) = source(vec![]).await;
    let bad = SourceFilter {
        statuses: vec!["a\nb".into()],
        ..filter()
    };
    assert!(s.search(&bad, NOW).await.is_err());
    assert!(f.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn search_paginates_three_pages() {
    let (f, s) = source(vec![
        page(vec![issue("ENG-1"), issue("ENG-2")], Some("c1")),
        page(vec![issue("ENG-3")], Some("c2")),
        page(vec![issue("ENG-4")], None),
    ])
    .await;
    let ts = s.search(&filter(), NOW).await.unwrap();
    let keys: Vec<_> = ts.iter().map(|t| t.key.to_string()).collect();
    assert_eq!(keys, ["ENG-1", "ENG-2", "ENG-3", "ENG-4"]);
    let cursors: Vec<Value> = f
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.1["variables"]["cursor"].clone())
        .collect();
    assert_eq!(cursors, [json!(null), json!("c1"), json!("c2")]);
}

#[tokio::test]
async fn failures_yield_no_partial_result() {
    let cases: Vec<(&str, Vec<Reply>, &str)> = vec![
        (
            "graphql errors on page 2",
            vec![
                page(vec![issue("ENG-1")], Some("c1")),
                Reply::Json(json!({"errors": [{"message": "Boom: bad thing"}], "data": null})),
            ],
            "Linear GraphQL error: Boom: bad thing",
        ),
        (
            "401",
            vec![Reply::Status(401, "nope")],
            "Linear API returned HTTP 401",
        ),
        (
            "500",
            vec![Reply::Status(500, "x")],
            "Linear API returned HTTP 500",
        ),
        ("429", vec![Reply::Status(429, "slow down")], "rate limit"),
        (
            "ratelimited code on 400",
            vec![Reply::Raw(
                br#"{"errors":[{"message":"x","extensions":{"code":"RATELIMITED"}}]}"#.to_vec(),
            )],
            "rate limit",
        ),
        (
            "ratelimited code on 200",
            vec![Reply::Json(
                json!({"errors": [{"message": "x", "extensions": {"code": "RATELIMITED"}}]}),
            )],
            "rate limit",
        ),
        ("not json", vec![Reply::Raw(b"<html>".to_vec())], "not JSON"),
    ];
    for (name, script, expect) in cases {
        let (_f, s) = source(script).await;
        let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
        assert!(e.contains(expect), "{name}: {e}");
        assert!(!e.contains(KEY) && !e.contains("127.0.0.1"), "{name}: {e}");
    }
}

#[tokio::test]
async fn long_tracker_message_is_truncated() {
    let msg = "m".repeat(1000);
    let (_f, s) = source(vec![Reply::Json(json!({"errors": [{"message": msg}]}))]).await;
    let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
    assert!(e.len() < 320, "{}", e.len());
}

#[tokio::test]
async fn key_echoed_by_tracker_is_scrubbed() {
    let (_f, s) = source(vec![Reply::Json(
        json!({"errors": [{"message": format!("bad key {KEY}")}]}),
    )])
    .await;
    let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
    assert!(!e.contains(KEY) && e.contains("[redacted]"), "{e}");
}

#[tokio::test]
async fn mapping() {
    let mut a = issue("ENG-1");
    a["priority"] = json!(1);
    a["state"] = json!({"name": "Backlog", "type": "backlog"});
    let mut b = issue("ENG-2");
    b["priority"] = json!(4);
    b["url"] = json!("https://evil.example/ENG-2");
    b["state"] = json!({"name": "Done", "type": "completed"});
    b["completedAt"] = json!("2026-01-01T00:00:01.500Z");
    b["assignee"] = json!(null);
    b["description"] = json!(null);
    let mut c = issue("ENG-3");
    c["priority"] = json!(0);
    c["state"] = json!({"name": "Canceled", "type": "canceled"});
    let (_f, s) = source(vec![page(vec![a, b, c], None)]).await;
    let ts = s.search(&filter(), NOW).await.unwrap();
    assert_eq!(ts[0].status_category, StatusCategory::Todo);
    assert_eq!(ts[0].priority, 10);
    assert_eq!(ts[0].project, "ENG");
    assert_eq!(ts[0].summary, "title ENG-1");
    assert_eq!(ts[0].status, "Backlog");
    assert_eq!(ts[0].updated_stamp, "2026-01-01T10:00:00.123Z");
    assert_eq!(ts[0].url, "https://linear.app/acme/issue/ENG-1/x");
    assert_eq!(ts[0].assignee.as_deref(), Some("Ada"));
    assert_eq!(ts[0].description, "desc **md**");
    assert_eq!(ts[1].status_category, StatusCategory::Done);
    assert_eq!(ts[1].priority, -5);
    assert_eq!(ts[1].url, "", "non-linear.app url dropped");
    assert_eq!(ts[1].resolved_at, Some(NOW + 1500));
    assert_eq!(ts[1].assignee, None);
    assert_eq!(ts[1].description, "");
    assert_eq!(ts[2].status_category, StatusCategory::Done);
    assert_eq!(ts[2].priority, 0);
    assert_eq!(ts[2].resolved_at, None);
}

#[tokio::test]
async fn malformed_identifier_fails_the_pass() {
    let mut bad = issue("ENG-1");
    bad["identifier"] = json!("nope");
    let (_f, s) = source(vec![page(vec![issue("ENG-2"), bad], None)]).await;
    assert!(s.search(&filter(), NOW).await.is_err());
}

fn comments_page(nodes: Vec<Value>, next: Option<&str>) -> Reply {
    Reply::Json(json!({"data": {"issue": {"comments": {"nodes": nodes,
        "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}}}}}))
}

#[tokio::test]
async fn comments_paged_and_mapped() {
    let (f, s) = source(vec![
        comments_page(
            vec![
                json!({"id": "c2", "body": "second", "createdAt": "2026-01-01T00:00:02.000Z",
                "user": {"id": "u9", "name": "Bea"}}),
            ],
            Some("cur"),
        ),
        comments_page(
            vec![json!({"id": "c1", "body": "first", "createdAt": "2026-01-01T00:00:01.250Z", "user": null})],
            None,
        ),
    ])
    .await;
    let key = TicketKey::parse("ENG-7").unwrap();
    let cs = s.comments(&key).await.unwrap();
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].id, "c1");
    assert_eq!(cs[0].order_key, NOW + 1250);
    assert_eq!(cs[0].created_at, NOW + 1250);
    assert_eq!(cs[0].author_id, "", "missing user");
    assert_eq!(cs[1].author_id, "u9");
    assert_eq!(cs[1].author_name, "Bea");
    assert_eq!(cs[1].body, "second");
    let reqs = f.requests.lock().unwrap();
    assert_eq!(reqs[0].1["query"], COMMENTS_QUERY);
    assert_eq!(reqs[0].1["variables"], json!({"id": "ENG-7", "cursor": null}));
    assert_eq!(reqs[1].1["variables"]["cursor"], "cur");
}

#[tokio::test]
async fn comments_error_is_an_error() {
    let (_f, s) = source(vec![
        comments_page(vec![], Some("x")),
        Reply::Json(json!({"errors": [{"message": "Entity not found"}]})),
    ])
    .await;
    let key = TicketKey::parse("ENG-7").unwrap();
    assert!(s.comments(&key).await.is_err());
}

#[tokio::test]
async fn statuses_distinct() {
    let (f, s) = source(vec![Reply::Json(json!({"data": {"workflowStates": {"nodes": [
        {"name": "Todo"}, {"name": "Done"}, {"name": "Todo"}]}}}))])
    .await;
    assert_eq!(s.known_statuses().await.unwrap(), ["Todo", "Done"]);
    assert_eq!(f.requests.lock().unwrap()[0].1["query"], STATUSES_QUERY);
}

#[test]
fn config_wiring() {
    let none = Config::from_lookup(|k| (k == "HOME").then(|| "/home/u".to_owned())).unwrap();
    assert!(from_config(&none).unwrap().is_none());
    let m: HashMap<&str, &str> = HashMap::from([("HOME", "/home/u"), ("CORVUS_LINEAR_API_KEY", KEY)]);
    let c = Config::from_lookup(|k| m.get(k).map(|s| (*s).to_owned())).unwrap();
    let s = from_config(&c).unwrap().unwrap();
    assert_eq!(s.id(), SourceId::Linear);
    assert_eq!(s.site(), "linear.app");
    assert_eq!(s.pool(), "default");
    assert_eq!(s.poll_interval(), Duration::from_secs(60));
    assert!(s.ignored_authors().is_empty());
    assert_eq!(s.default_filter().done_days, 7);
    assert!(!format!("{s:?}").contains(KEY));

    let custom =
        LinearSource::with_endpoint_for_tests("http://127.0.0.1:1/graphql", &cfg(), Limits::default())
            .unwrap();
    assert_eq!(custom.poll_interval(), Duration::from_secs(42));
    assert_eq!(custom.pool(), "lin");
    assert_eq!(custom.ignored_authors(), ["u1", "u2"]);
}

#[test]
fn non_https_remote_endpoints_refused() {
    for url in [
        "http://api.linear.app/graphql",
        "http://example.com/x",
        "https://example.com/graphql",
    ] {
        assert!(
            LinearSource::with_endpoint_for_tests(url, &cfg(), Limits::default()).is_err(),
            "{url}"
        );
    }
}

#[tokio::test]
async fn redirects_not_followed() {
    let (f, s) = source(vec![Reply::Redirect]).await;
    let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
    assert!(e.contains("HTTP 302"), "{e}");
    assert_eq!(*f.other_hits.lock().unwrap(), 0);
}

#[tokio::test]
async fn timeout_enforced() {
    let limits = Limits {
        total_timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let (_f, s) = source_with(vec![Reply::Sleep(Duration::from_secs(3))], limits).await;
    let t = std::time::Instant::now();
    let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
    assert!(e.contains("timed out"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn body_cap_enforced() {
    let limits = Limits {
        max_body_bytes: 1024,
        ..Limits::default()
    };
    let big = format!(r#"{{"data":{{"pad":"{}"}}}}"#, "x".repeat(5000));
    let (_f, s) = source_with(vec![Reply::Raw(big.into_bytes())], limits).await;
    let e = s.search(&filter(), NOW).await.unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
}

#[derive(Clone)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs (once, process-wide, so every test's events land here) a subscriber writing to a shared buffer.
fn log_buffer() -> Buf {
    static BUF: std::sync::OnceLock<Buf> = std::sync::OnceLock::new();
    BUF.get_or_init(|| {
        let buf = Buf(Arc::default());
        let b2 = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || b2.clone())
            .finish();
        tracing::subscriber::set_global_default(sub).unwrap();
        buf
    })
    .clone()
}

#[tokio::test]
async fn key_never_in_logs_or_errors() {
    let (_f, s) = source(vec![
        Reply::Status(401, "bad"),
        Reply::Status(429, "slow"),
        Reply::Json(json!({"errors": [{"message": format!("echo {KEY}")}]})),
    ])
    .await;
    let mut errs = Vec::new();
    for _ in 0..3 {
        errs.push(s.search(&filter(), NOW).await.unwrap_err().to_string());
    }
    let logs = String::from_utf8(log_buffer().0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("Linear request failed"),
        "something was logged: {logs}"
    );
    assert!(logs.contains("Linear rate-limit headers"), "{logs}");
    assert!(!logs.contains(KEY) && !logs.contains("SECRETKEY"), "{logs}");
    for e in errs {
        assert!(!e.contains(KEY) && !e.contains("SECRETKEY"), "{e}");
    }
}
