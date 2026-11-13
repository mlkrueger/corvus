#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvus_core::sources::WorkSource;
use corvus_domain::TicketKey;
use serde_json::json;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN: &str = "s3cr3t-token-XYZ";
mod common;
use common::*;

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
impl<'a> MakeWriter<'a> for Buf {
    type Writer = Buf;
    fn make_writer(&'a self) -> Buf {
        self.clone()
    }
}

#[tokio::test]
async fn token_never_appears_in_errors_or_logs() {
    let buf = Buf::default();
    let sub = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _g = tracing::subscriber::set_default(sub);
    let (base, _) = serve(|r| {
        if r.path.contains("/comment") {
            Resp::json(json!({"comments": [{"id": "x"}], "total": 1}))
        } else if r.path.contains("/status") {
            Resp::status(401)
        } else {
            let mut x = Resp::status(302);
            x.headers
                .push(("location", format!("http://user:{TOKEN}@127.0.0.1:1/")));
            x
        }
    })
    .await;
    let s = source(&base);
    let mut errs = vec![
        s.search(&filter(), 0).await.unwrap_err().to_string(),
        s.known_statuses().await.unwrap_err().to_string(),
        s.comments(&TicketKey::parse("AA-1").unwrap())
            .await
            .unwrap_err()
            .to_string(),
    ];
    // Connection failure too (nothing listens on port 1).
    let dead = source_with(
        "http://127.0.0.1:1",
        Duration::from_millis(300),
        Duration::from_secs(1),
        1000,
    );
    errs.push(dead.search(&filter(), 0).await.unwrap_err().to_string());
    errs.push(format!("{s:?}"));
    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("Jira API"), "warnings are logged: {logs}");
    let b64 = "YWRhQGV4YW1wbGUudGVzdDpzM2NyM3QtdG9rZW4tWFla";
    for text in errs.iter().chain([&logs]) {
        assert!(
            !text.contains(TOKEN) && !text.contains(b64) && !text.to_lowercase().contains("authorization"),
            "leak in: {text}"
        );
    }
    assert!(
        errs[0].contains("HTTP 302") && errs[1].contains("HTTP 401"),
        "{errs:?}"
    );
}
