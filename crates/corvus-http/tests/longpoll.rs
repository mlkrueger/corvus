#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::{Req, env, post, register, send};
use serde_json::{Value, json};

#[tokio::test]
async fn returns_promptly_when_another_connection_posts() {
    let e = env();
    register(&e.app, "alice").await;
    let sid = register(&e.app, "bob").await;
    let th = post(&e.app, "/api/threads", &json!({"creator": "alice", "title": "t"}))
        .await
        .json();

    let app = e.app.clone();
    let started = Instant::now();
    let waiter = tokio::spawn(async move {
        send(
            &app,
            Req::get(&format!("/api/inbox/bob/wait?timeout=30&session_id={sid}")).token(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiter.is_finished(), "must be waiting");

    // Another connection (a separate Corvus on the same file) posts.
    let other = corvus_core::Corvus::new(e.t.services(), corvus_core::hermod::Hermod::new(&e.t.path));
    let body: corvus_core::api::PostMessage = serde_json::from_value(
        json!({"thread_id": th["id"], "from": "alice", "to": "bob", "body": "wake up"}),
    )
    .unwrap();
    other.run(move |s| s.muninn().post_message(&body)).await.unwrap();

    let r = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, StatusCode::OK);
    let v: Value = r.json();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["body"], "wake up");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn returns_empty_at_timeout() {
    let e = env();
    register(&e.app, "bob").await;
    let started = Instant::now();
    let r = send(&e.app, Req::get("/api/inbox/bob/wait?timeout=1").token()).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), json!([]));
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(4),
        "{took:?}"
    );
}

#[tokio::test]
async fn client_disconnect_stops_the_watcher() {
    let e = env();
    register(&e.app, "bob").await;
    assert!(!e.hermod.watcher_running());

    let app = e.app.clone();
    let waiter =
        tokio::spawn(async move { send(&app, Req::get("/api/inbox/bob/wait?timeout=60").token()).await });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !e.hermod.watcher_running() {
        assert!(Instant::now() < deadline, "watcher never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The client goes away: the request future is dropped mid-wait.
    waiter.abort();
    let _ = waiter.await;

    let deadline = Instant::now() + Duration::from_secs(1) + Duration::from_millis(500);
    while e.hermod.watcher_running() {
        assert!(Instant::now() < deadline, "watcher still running: waiter leaked");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
