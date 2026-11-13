#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{TOKEN, env};
use corvus_http::ApiToken;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

async fn raw(port: u16, request: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn serve_binds_loopback_only_and_shuts_down() {
    let e = env();
    let port = free_port();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(corvus_http::serve(
        e.corvus.clone(),
        ApiToken::new(TOKEN).unwrap(),
        port,
        async move {
            let _ = stop_rx.await;
        },
    ));
    // Wait for the bind.
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let auth = format!(
        "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    );
    let res = raw(port, &auth).await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert!(res.to_lowercase().contains("cache-control: no-store"), "{res}");
    assert!(res.contains(r#""ok":true"#), "{res}");

    let noauth = format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    assert!(raw(port, &noauth).await.starts_with("HTTP/1.1 401"));
    let evil = format!(
        "GET /api/health HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    );
    assert!(raw(port, &evil).await.starts_with("HTTP/1.1 403"));

    // Chunked body over the cap, no Content-Length: rejected with 413.
    let chunk = "a".repeat(200 * 1024);
    let chunked = format!(
        "POST /api/sessions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{n:x}\r\n{chunk}\r\n{n:x}\r\n{chunk}\r\n0\r\n\r\n",
        n = chunk.len()
    );
    let res = raw(port, &chunked).await;
    assert!(res.starts_with("HTTP/1.1 413"), "{}", &res[..res.len().min(200)]);

    // Loopback only: not reachable over IPv6.
    assert!(
        TcpStream::connect(("::1", port)).await.is_err(),
        "must not listen on ::1"
    );

    stop_tx.send(()).unwrap();
    let r = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap();
    assert!(r.is_ok());
    assert!(
        TcpStream::connect(("127.0.0.1", port)).await.is_err(),
        "listener closed"
    );
}

#[tokio::test]
async fn serve_fails_when_port_is_taken() {
    let e = env();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let r = corvus_http::serve(
        e.corvus.clone(),
        ApiToken::new(TOKEN).unwrap(),
        port,
        std::future::pending(),
    )
    .await;
    assert!(r.is_err());
}
