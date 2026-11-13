#![allow(clippy::unwrap_used)]

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{Req, TOKEN, env, register, send};
use serde_json::json;

#[derive(Clone, Default)]
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

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
    type Writer = Buf;

    fn make_writer(&'a self) -> Buf {
        self.clone()
    }
}

#[tokio::test]
async fn logs_contain_no_token_body_or_query() {
    let buf = Buf::default();
    // Global, so the blocking pool and watcher threads are captured too. This file has one test.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish(),
    )
    .unwrap();

    let e = env();
    register(&e.app, "alice").await;
    let th = send(
        &e.app,
        Req::post("/api/threads")
            .token()
            .json(&json!({"creator": "alice", "title": "t"})),
    )
    .await
    .json();
    let r = send(
        &e.app,
        Req::post("/api/messages")
            .token()
            .json(&json!({"thread_id": th["id"], "from": "alice", "body": "BODY-MARKER-9f3a"})),
    )
    .await;
    assert!(r.status.is_success(), "{}", String::from_utf8_lossy(&r.body));
    let r = send(
        &e.app,
        Req::get("/api/messages?agent=QUERY-MARKER-77&limit=3").token(),
    )
    .await;
    assert!(r.status.is_success());
    // Failed auth with a secret-looking token too.
    send(
        &e.app,
        Req::get("/api/health").header("authorization", "Bearer WRONG-TOKEN-MARKER-55"),
    )
    .await;

    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("route=/api/messages"),
        "request lines missing:\n{logs}"
    );
    assert!(
        logs.contains("status=200") && logs.contains("duration_ms="),
        "{logs}"
    );
    assert!(logs.contains("status=401"), "{logs}");
    for secret in [
        TOKEN,
        "BODY-MARKER-9f3a",
        "QUERY-MARKER-77",
        "WRONG-TOKEN-MARKER-55",
        "limit=3",
        "Bearer",
    ] {
        assert!(!logs.contains(secret), "logs leak {secret:?}:\n{logs}");
    }
}
