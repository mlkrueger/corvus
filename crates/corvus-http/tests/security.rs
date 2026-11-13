#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashSet;

use axum::http::StatusCode;
use common::{PORT, Req, TOKEN, env, send};
use corvus_http::{ApiToken, route_table};

const UUID: &str = "0190f2a0-0000-7000-8000-000000000000";

/// Fills `{param}` placeholders with well-formed values; the long-poll route gets `timeout=0` so it returns.
fn sample(path: &str) -> String {
    let mut p = path
        .replace("{agent}", "alice")
        .replace("{source}", "jira")
        .replace("{id}", UUID);
    if path.starts_with("/api/messages/{id}") {
        p = p.replace(UUID, "1");
    }
    if path.ends_with("/wait") {
        p.push_str("?timeout=0");
    }
    p
}

#[test]
fn route_table_is_complete_and_unique() {
    let table = route_table();
    assert!(table.len() >= 35, "table suspiciously small: {}", table.len());
    let set: HashSet<_> = table.iter().collect();
    assert_eq!(set.len(), table.len(), "duplicate routes");
    for (m, p) in table {
        assert!(p.starts_with("/api/"), "{p}");
        assert!(["GET", "POST"].contains(m), "{m}");
        assert!(
            !p.starts_with("/api/sources"),
            "sources routes belong to plan 0008"
        );
    }
}

#[tokio::test]
async fn every_route_demands_the_token() {
    let e = env();
    for (method, path) in route_table() {
        let uri = sample(path);
        let none = send(&e.app, Req::new(method, &uri)).await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED, "{method} {path}: no token");
        assert_eq!(none.code(), "unauthorized", "{method} {path}");

        let wrong = send(
            &e.app,
            Req::new(method, &uri).header("authorization", "Bearer nope"),
        )
        .await;
        assert_eq!(
            wrong.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path}: wrong token"
        );

        let near = send(
            &e.app,
            Req::new(method, &uri).header("authorization", &format!("Bearer {TOKEN}x")),
        )
        .await;
        assert_eq!(
            near.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path}: near-miss token"
        );

        let basic = send(
            &e.app,
            Req::new(method, &uri).header("authorization", &format!("Basic {TOKEN}")),
        )
        .await;
        assert_eq!(
            basic.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path}: wrong scheme"
        );

        let ok = send(&e.app, Req::new(method, &uri).token()).await;
        assert!(
            ok.status != StatusCode::UNAUTHORIZED && ok.status != StatusCode::FORBIDDEN,
            "{method} {path}: with token got {}",
            ok.status
        );
        // The route is really served (not an unmatched 404 or a 405).
        assert_ne!(ok.status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        assert!(
            !(ok.status == StatusCode::NOT_FOUND && ok.body.is_empty()),
            "{method} {path}: not routed"
        );
    }
}

#[tokio::test]
async fn health_is_not_exempt() {
    let e = env();
    assert_eq!(
        send(&e.app, Req::get("/api/health")).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&e.app, Req::get("/api/health").token()).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn no_pages_docs_or_schema_routes() {
    let e = env();
    for path in [
        "/ui",
        "/ui/",
        "/docs",
        "/openapi.json",
        "/api",
        "/api/",
        "/schema",
        "/",
        "/api/sources",
    ] {
        let r = send(&e.app, Req::get(path).token()).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{path}");
        assert!(r.body.is_empty(), "{path}: bare 404");
        // Unauthenticated callers learn nothing either.
        assert_eq!(
            send(&e.app, Req::get(path)).await.status,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
}

#[tokio::test]
async fn host_allow_list() {
    let e = env();
    let with_host = |h: Option<&str>| {
        let r = Req::bare("GET", "/api/health").token();
        match h {
            Some(h) => r.host(h),
            None => r,
        }
    };
    for bad in [
        Some("evil.example"),
        Some("127.0.0.1.evil.example"),
        Some("localhost.evil"),
        Some("127.0.0.1:1"),
        Some("localhost:80"),
        Some("[::1]:99"),
        Some("evil.example:47001"),
        Some("127.0.0.1:047001"),
        Some("127.0.0.1:"),
        Some(""),
        None,
    ] {
        let r = send(&e.app, with_host(bad)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "host {bad:?}");
        assert_eq!(r.code(), "forbidden_host", "host {bad:?}");
    }
    for good in [
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{PORT}"),
        "localhost".to_owned(),
        format!("localhost:{PORT}"),
        "LOCALHOST".to_owned(),
        "[::1]".to_owned(),
        format!("[::1]:{PORT}"),
    ] {
        let r = send(&e.app, with_host(Some(&good))).await;
        assert_eq!(r.status, StatusCode::OK, "host {good}");
    }
}

#[tokio::test]
async fn host_is_checked_before_auth() {
    let e = env();
    let r = send(&e.app, Req::bare("GET", "/api/health").host("evil.example")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_cors_headers_anywhere() {
    let e = env();
    let has_cors = |r: &common::Reply| {
        r.headers
            .keys()
            .any(|k| k.as_str().starts_with("access-control-"))
    };

    for token in [false, true] {
        let mut req = Req::new("OPTIONS", "/api/tasks")
            .header("origin", "https://evil.example")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "authorization");
        if token {
            req = req.token();
        }
        let r = send(&e.app, req).await;
        assert!(
            !has_cors(&r),
            "preflight (token={token}) got CORS headers: {:?}",
            r.headers
        );
        assert!(!r.status.is_success(), "preflight must not be approved");
    }
    for req in [
        Req::get("/api/health")
            .token()
            .header("origin", "https://evil.example"),
        Req::get("/api/health").header("origin", "https://evil.example"),
        Req::get("/nope").token(),
    ] {
        assert!(!has_cors(&send(&e.app, req).await));
    }
}

#[tokio::test]
async fn security_headers_on_every_response() {
    let e = env();
    let cases = [
        ("200", Req::get("/api/health").token()),
        ("401", Req::get("/api/health")),
        ("403", Req::get("/api/health").token().header("x", "y")),
        ("404 service", Req::get(&format!("/api/tasks/{UUID}")).token()),
        ("404 bare", Req::get("/nothing").token()),
        ("422", Req::get("/api/tasks/not-a-uuid").token()),
        ("405", Req::new("DELETE", "/api/health").token()),
    ];
    for (name, req) in cases {
        let r = if name == "403" {
            // A bad host.
            send(
                &e.app,
                Req::bare("GET", "/api/health").token().host("evil.example"),
            )
            .await
        } else {
            send(&e.app, req).await
        };
        assert_eq!(
            r.headers.get("cache-control").unwrap(),
            "no-store",
            "{name} ({})",
            r.status
        );
        assert_eq!(
            r.headers.get("x-content-type-options").unwrap(),
            "nosniff",
            "{name}"
        );
    }
}

#[test]
fn api_token_length_and_debug() {
    assert!(ApiToken::new(&"a".repeat(31)).is_err());
    assert!(ApiToken::new("").is_err());
    let ok = ApiToken::new(&"a".repeat(32)).unwrap();
    assert!(ok.verify(&"a".repeat(32)));
    assert!(!ok.verify(&"a".repeat(31)));
    assert!(!ok.verify(&"b".repeat(32)));
    assert!(!ok.verify(""));
    let dbg = format!("{ok:?}");
    assert!(!dbg.contains("aaaa"), "{dbg}");
    // Multi-byte characters count as characters, not bytes.
    assert!(ApiToken::new(&"é".repeat(31)).is_err());
    assert!(ApiToken::new(&"é".repeat(32)).is_ok());
}
