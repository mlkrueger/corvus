#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use common::{Req, env, get, post, register, send};
use corvus_domain::CorvusError;
use serde_json::{Value, json};

const UUID: &str = "0190f2a0-0000-7000-8000-000000000000";

async fn mapped(err: CorvusError) -> (StatusCode, Value) {
    let res = corvus_http::ApiError::from(err).into_response();
    let (parts, body) = res.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    (parts.status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn every_corvus_error_variant_maps_per_the_spec() {
    let cases = [
        (CorvusError::NotFound("x".into()), 404, "not_found"),
        (CorvusError::Conflict("x".into()), 409, "conflict"),
        (CorvusError::SessionNotActive, 409, "session_not_active"),
        (CorvusError::Validation("x".into()), 422, "validation"),
        (CorvusError::Unauthorized, 401, "unauthorized"),
        (CorvusError::Busy, 503, "busy"),
        (CorvusError::Internal("secret detail".into()), 500, "internal"),
    ];
    for (err, status, code) in cases {
        let (s, body) = mapped(err).await;
        assert_eq!(s.as_u16(), status, "{code}");
        assert_eq!(body["error"]["code"], code);
        assert!(body["error"]["message"].is_string(), "{code}");
        assert_eq!(body.as_object().unwrap().len(), 1, "only the error key");
    }
}

#[tokio::test]
async fn internal_errors_leak_no_detail() {
    let (s, body) = mapped(CorvusError::Internal(
        "rusqlite: no such table: /home/u/corvus.db".into(),
    ))
    .await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    let text = body.to_string();
    assert!(
        !text.contains("rusqlite") && !text.contains("corvus.db") && !text.contains("table"),
        "{text}"
    );
}

#[tokio::test]
async fn service_errors_surface_end_to_end() {
    let e = env();
    // not_found
    let r = get(&e.app, &format!("/api/tasks/{UUID}")).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    let r = get(&e.app, &format!("/api/threads/{UUID}")).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    // validation from the service (empty title)
    let r = post(&e.app, "/api/tasks", &json!({"creator": "alice", "title": ""})).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation")
    );

    // session_not_active: claim with an ended session.
    let sid = register(&e.app, "alice").await;
    assert_eq!(
        post(&e.app, &format!("/api/sessions/{sid}/end"), &json!({}))
            .await
            .status,
        StatusCode::OK
    );
    let r = post(&e.app, "/api/tasks/claim", &json!({"session_id": sid})).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::CONFLICT, "session_not_active"),
        "{:?}",
        r.json()
    );

    // conflict: a done task cannot be cancelled.
    let sid = register(&e.app, "bob").await;
    let task = post(&e.app, "/api/tasks", &json!({"creator": "bob", "title": "t"}))
        .await
        .json();
    let tid = task["id"].as_str().unwrap();
    assert_eq!(
        post(&e.app, "/api/tasks/claim", &json!({"session_id": sid}))
            .await
            .status,
        StatusCode::OK
    );
    let done = post(
        &e.app,
        &format!("/api/tasks/{tid}/complete"),
        &json!({"session_id": sid}),
    )
    .await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.json());
    let r = post(&e.app, &format!("/api/tasks/{tid}/cancel"), &json!({"by": "bob"})).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::CONFLICT, "conflict"),
        "{:?}",
        r.json()
    );
}

#[tokio::test]
async fn malformed_requests_are_422_with_the_error_shape() {
    let e = env();
    let json_422 = |r: &common::Reply, what: &str| {
        assert_eq!(
            r.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{what}: {}",
            String::from_utf8_lossy(&r.body)
        );
        assert_eq!(r.code(), "validation", "{what}");
        assert!(r.json()["error"]["message"].is_string(), "{what}");
    };

    // malformed JSON
    let r = send(
        &e.app,
        Req::post("/api/sessions")
            .token()
            .raw("application/json", b"{not json".to_vec()),
    )
    .await;
    json_422(&r, "malformed json");
    // missing content type
    let r = send(
        &e.app,
        Req::post("/api/sessions")
            .token()
            .raw("text/plain", b"{}".to_vec()),
    )
    .await;
    json_422(&r, "wrong content-type");
    // unknown field
    let r = post(&e.app, "/api/sessions", &json!({"client_name": "t", "bogus": 1})).await;
    json_422(&r, "unknown field");
    // missing required field
    let r = post(&e.app, "/api/sessions", &json!({})).await;
    json_422(&r, "missing field");
    // wrong type
    let r = post(&e.app, "/api/sessions", &json!({"client_name": 5})).await;
    json_422(&r, "wrong type");

    // bad path params
    json_422(&get(&e.app, "/api/tasks/not-a-uuid").await, "task uuid");
    json_422(&get(&e.app, "/api/threads/123/messages").await, "thread uuid");
    json_422(
        &post(&e.app, "/api/sessions/zzz/heartbeat", &json!({})).await,
        "session uuid",
    );
    json_422(
        &post(&e.app, "/api/tasks/zzz/renew", &json!({"session_id": UUID})).await,
        "task uuid renew",
    );
    json_422(&get(&e.app, "/api/inbox/Not%20Valid!").await, "agent name");
    json_422(
        &get(&e.app, "/api/inbox/-bad/wait?timeout=0").await,
        "agent name wait",
    );
    json_422(
        &get(&e.app, "/api/bootstrap/bad%2Fname").await,
        "agent name bootstrap",
    );
    json_422(
        &post(&e.app, "/api/messages/abc/ack", &json!({"agent": "alice"})).await,
        "message id",
    );

    // bad query
    json_422(&get(&e.app, "/api/tasks?limit=abc").await, "limit");
    json_422(&get(&e.app, "/api/tasks?status=bogus").await, "status");
    json_422(&get(&e.app, "/api/tasks?unknown=1").await, "unknown query");
    json_422(&get(&e.app, "/api/inbox/alice/wait?timeout=abc").await, "timeout");
    json_422(&get(&e.app, "/api/digests/latest").await, "missing scope");
}

#[tokio::test]
async fn bodies_over_256_kib_are_rejected() {
    let e = env();
    let big = json!({"client_name": "x".repeat(300 * 1024)});
    let r = post(&e.app, "/api/sessions", &big).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.code(), "payload_too_large");
    assert_eq!(r.headers.get("cache-control").unwrap(), "no-store");

    // Just under the cap is read (and then fails validation, not size).
    let ok = json!({"client_name": "x".repeat(250 * 1024)});
    let r = post(&e.app, "/api/sessions", &ok).await;
    assert_ne!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}
