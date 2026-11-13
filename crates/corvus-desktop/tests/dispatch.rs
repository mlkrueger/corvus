#![allow(clippy::unwrap_used)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, header};
use corvus_core::Corvus;
use corvus_core::hermod::Hermod;
use corvus_core::testing::TestDb;
use corvus_desktop::ipc::{ApiResponse, dispatch};
use serde_json::{Value, json};
use tower::ServiceExt;

fn router(t: &TestDb) -> Router {
    corvus_http::data_router(Corvus::new(t.services(), Hermod::new(&t.path)))
}

/// What a request returns, from either the IPC path or a direct call into the router.
trait Caller {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value);
}

struct Ipc(Router);
struct Direct(Router);

impl Caller for Ipc {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let ApiResponse { status, body } = dispatch(&self.0, method, path, body).await.unwrap();
        (status, body)
    }
}

impl Caller for Direct {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = Request::builder().method(method).uri(path);
        let payload = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&v).unwrap())
            }
            None => Body::empty(),
        };
        let res = self.0.clone().oneshot(req.body(payload).unwrap()).await.unwrap();
        let status = res.status().as_u16();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }
}

/// Ids and timestamps differ between two databases; replace them so the rest compares exactly.
fn normalize(v: &Value) -> Value {
    match v {
        Value::String(s) if is_uuid(s) => json!("<uuid>"),
        Value::String(s) if s.len() >= 20 && s.ends_with('Z') && s.as_bytes()[10] == b'T' => json!("<ts>"),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), normalize(v))).collect()),
        other => other.clone(),
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

fn label_path(path: &str) -> String {
    path.split('/')
        .map(|seg| match seg {
            UNKNOWN => "<unknown>",
            s if is_uuid(s) => "<uuid>",
            s => s,
        })
        .collect::<Vec<_>>()
        .join("/")
}

const UNKNOWN: &str = "0190f2a0-0000-7000-8000-000000000000";

/// One scripted session covering every route group; returns `(label, status, normalized body)` per step.
async fn scenario<C: Caller>(c: &C) -> Vec<(String, u16, Value)> {
    let mut out = Vec::new();
    macro_rules! step {
        ($m:expr, $p:expr, $b:expr) => {{
            let path: String = $p;
            let (status, body) = c.call($m, &path, $b).await;
            out.push((format!("{} {}", $m, label_path(&path)), status, normalize(&body)));
            body
        }};
    }
    let id = |v: &Value| v["id"].as_str().unwrap().to_owned();

    step!("GET", "/api/health".into(), None);
    let alice = id(&step!(
        "POST",
        "/api/sessions".into(),
        Some(json!({"agent_name": "alice", "client_name": "t"}))
    ));
    step!(
        "POST",
        "/api/sessions".into(),
        Some(json!({"agent_name": "bob", "client_name": "t"}))
    );
    step!("GET", "/api/sessions".into(), None);
    step!(
        "POST",
        format!("/api/sessions/{alice}/heartbeat"),
        Some(json!({}))
    );

    let task = id(&step!(
        "POST",
        "/api/tasks".into(),
        Some(json!({"creator": "alice", "title": "one", "body": "do it"}))
    ));
    step!("GET", "/api/tasks".into(), None);
    step!("GET", "/api/tasks?status=open&limit=5".into(), None);
    step!("GET", format!("/api/tasks/{task}"), None);
    step!(
        "POST",
        "/api/tasks/claim".into(),
        Some(json!({"session_id": alice}))
    );
    step!(
        "POST",
        format!("/api/tasks/{task}/renew"),
        Some(json!({"session_id": alice}))
    );
    step!(
        "POST",
        format!("/api/tasks/{task}/complete"),
        Some(json!({"session_id": alice, "result_summary": "ok"}))
    );

    let thread = id(&step!(
        "POST",
        "/api/threads".into(),
        Some(json!({"creator": "alice", "title": "talk"}))
    ));
    step!(
        "POST",
        "/api/messages".into(),
        Some(
            json!({"thread_id": thread, "from": "alice", "to": "bob", "body": "hello", "session_id": alice})
        )
    );
    step!(
        "GET",
        "/api/messages?limit=10&agent=bob&unread_only=true".into(),
        None
    );
    step!("GET", format!("/api/threads/{thread}/messages?limit=10"), None);
    step!(
        "POST",
        "/api/messages/direct".into(),
        Some(json!({"from": "alice", "to": "bob", "body": "psst"}))
    );
    step!("GET", "/api/inbox/bob?limit=10".into(), None);
    step!("GET", "/api/inbox/bob/wait?timeout=0".into(), None);
    step!(
        "POST",
        "/api/messages/1/ack".into(),
        Some(json!({"agent": "bob"}))
    );

    step!("GET", "/api/digests/latest?scope=global".into(), None);
    step!(
        "POST",
        "/api/digests".into(),
        Some(json!({"author": "alice", "scope": "global", "summary": "state"}))
    );
    step!("GET", "/api/digests".into(), None);
    step!(
        "POST",
        "/api/decisions".into(),
        Some(json!({"decided_by": "alice", "title": "use x", "decision": "x", "rationale": "because"}))
    );
    step!("GET", "/api/decisions?include_superseded=true".into(), None);
    step!("GET", "/api/bootstrap/alice".into(), None);
    step!("GET", "/api/overview".into(), None);
    step!("GET", "/api/stats".into(), None);
    step!("GET", "/api/events?limit=5".into(), None);

    // Error shapes.
    step!("GET", format!("/api/tasks/{UNKNOWN}"), None); // 404
    step!("POST", "/api/tasks".into(), Some(json!({"bogus": 1}))); // 422 (bad body)
    step!(
        "POST",
        "/api/tasks".into(),
        Some(json!({"creator": "alice", "title": ""}))
    ); // 422 (service)
    step!("GET", "/api/tasks/not-a-uuid".into(), None); // 422 (path)
    step!(
        "POST",
        format!("/api/tasks/{task}/renew"),
        Some(json!({"session_id": alice}))
    ); // 409 (done task)
    step!("GET", "/api/nope".into(), None); // router 404, empty body -> null
    out
}

#[tokio::test]
async fn dispatch_matches_a_direct_router_call_for_every_route_group() {
    let (a, b) = (TestDb::new(), TestDb::new());
    let via_ipc = scenario(&Ipc(router(&a))).await;
    let direct = scenario(&Direct(router(&b))).await;
    assert_eq!(via_ipc.len(), direct.len());
    for (i, d) in via_ipc.iter().zip(&direct) {
        assert_eq!(i, d, "mismatch for {}", i.0);
    }

    let statuses: Vec<u16> = via_ipc.iter().map(|s| s.1).collect();
    for want in [200, 404, 409, 422] {
        assert!(
            statuses.contains(&want),
            "scenario never produced {want}: {statuses:?}"
        );
    }
    let find = |label: &str| via_ipc.iter().find(|s| s.0 == label).unwrap();
    let not_found = find("GET /api/tasks/<unknown>");
    assert_eq!(
        (not_found.1, &not_found.2["error"]["code"]),
        (404, &json!("not_found"))
    );
    assert!(
        via_ipc
            .iter()
            .any(|s| s.0 == "POST /api/tasks" && s.1 == 422 && s.2["error"]["code"] == "validation")
    );
    assert_eq!(find("GET /api/nope").2, Value::Null, "an empty body is null");
    assert_eq!(find("GET /api/health").2["ok"], true);
}

#[tokio::test]
async fn dispatch_rejects_bad_requests_before_the_router() {
    let t = TestDb::new();
    let r = router(&t);
    for (m, p) in [
        ("DELETE", "/api/health"),
        ("PATCH", "/api/health"),
        ("GET", "/health"),
        ("GET", "/api"),
        ("GET", "api/health"),
        ("GET", "http://example.com/api/health"),
        ("GET", "//example.com/api/health"),
        ("GET", "/api/../etc/passwd"),
    ] {
        assert!(dispatch(&r, m, p, None).await.is_err(), "{m} {p}");
    }
}

#[tokio::test]
async fn dispatch_caps_the_request_body() {
    let t = TestDb::new();
    let r = router(&t);
    let big = json!({"creator": "alice", "title": "x", "body": "a".repeat(300 * 1024)});
    let res = dispatch(&r, "POST", "/api/tasks", Some(big)).await.unwrap();
    assert_eq!(res.status, 413);
    assert_eq!(res.body["error"]["code"], "payload_too_large");
}
