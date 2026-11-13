#![allow(clippy::unwrap_used)]

use std::time::Duration;

use corvus_core::Config;
use corvus_core::sources::WorkSource;
use corvus_domain::{SourceId, StatusCategory, TicketKey};
use serde_json::{Value, json};

const TOKEN: &str = "s3cr3t-token-XYZ";
mod common;
use common::*;
fn issue(key: &str, updated: &str) -> Value {
    json!({"key": key, "fields": {
        "summary": "Do it", "updated": updated,
        "status": {"name": "To Do", "statusCategory": {"key": "new"}},
        "priority": {"name": "High"},
        "project": {"key": key.split('-').next().unwrap()},
        "assignee": {"displayName": "Ada", "accountId": "acc1"},
        "resolutiondate": null,
        "description": {"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "hello"}]}]}
    }})
}

fn body_of(r: &Req) -> Value {
    serde_json::from_str(&r.body).unwrap()
}

#[tokio::test]
async fn search_follows_next_page_token_across_three_pages() {
    let (base, log) = serve(|r| {
        let b = body_of(r);
        match b.get("nextPageToken").and_then(Value::as_str) {
            None => {
                Resp::json(json!({"issues": [issue("AA-1", "u1")], "nextPageToken": "t2", "isLast": false}))
            }
            Some("t2") => {
                Resp::json(json!({"issues": [issue("AA-2", "u2")], "nextPageToken": "t3", "isLast": false}))
            }
            Some("t3") => Resp::json(json!({"issues": [issue("AA-3", "u3")], "isLast": true})),
            Some(_) => Resp::status(400),
        }
    })
    .await;
    let tickets = source(&base).search(&filter(), 0).await.unwrap();
    assert_eq!(
        tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
        ["AA-1", "AA-2", "AA-3"]
    );
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 3);
    assert!(
        log.iter()
            .all(|r| r.method == "POST" && r.path == "/rest/api/3/search/jql")
    );
    let first = body_of(&log[0]);
    assert!(
        first["jql"]
            .as_str()
            .unwrap()
            .starts_with("assignee = currentUser()")
    );
    assert_eq!(first["maxResults"], 50, "Jira caps search pages at 50");
    assert!(
        first["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "description")
    );
    assert!(first.get("nextPageToken").is_none());
    assert_eq!(body_of(&log[2])["nextPageToken"], "t3");
}

#[tokio::test]
async fn failing_page_fails_the_whole_search() {
    let (base, _) = serve(|r| match body_of(r).get("nextPageToken") {
        None => Resp::json(json!({"issues": [issue("AA-1", "u1")], "nextPageToken": "t2", "isLast": false})),
        Some(_) => Resp::status(500),
    })
    .await;
    let err = source(&base).search(&filter(), 0).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 500") && err.contains("search"), "{err}");
}

#[tokio::test]
async fn repeated_page_token_is_an_error_not_a_loop() {
    let (base, log) =
        serve(|_| Resp::json(json!({"issues": [], "nextPageToken": "same", "isLast": false}))).await;
    assert!(source(&base).search(&filter(), 0).await.is_err());
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn comments_page_with_start_at_until_total() {
    let comment = |id: u32| {
        json!({"id": id.to_string(), "author": {"accountId": "acc", "displayName": "Bea"},
               "created": "2026-01-02T03:04:05.000+0000",
               "body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": format!("c{id}")}]}]}})
    };
    let (base, log) = serve(move |r| {
        let start: u32 = r
            .path
            .split("startAt=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let items: Vec<Value> = (start..(start + 2).min(5)).map(|i| comment(1000 + i)).collect();
        Resp::json(json!({"comments": items, "total": 5, "startAt": start, "maxResults": 2}))
    })
    .await;
    let key = TicketKey::parse("AA-7").unwrap();
    let cs = source(&base).comments(&key).await.unwrap();
    assert_eq!(cs.len(), 5);
    assert_eq!(cs[0].order_key, 1000);
    assert_eq!(cs[4].order_key, 1004);
    assert_eq!(cs[0].id, "1000");
    assert_eq!(cs[0].author_id, "acc");
    assert_eq!(cs[0].author_name, "Bea");
    assert_eq!(cs[0].body, "c1000");
    assert_eq!(cs[0].created_at, 1_767_323_045_000);
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 3);
    assert!(log[0].path.starts_with("/rest/api/3/issue/AA-7/comment?"));
    assert!(log[1].path.contains("startAt=2") && log[2].path.contains("startAt=4"));
}

#[tokio::test]
async fn non_numeric_comment_id_fails_the_pass() {
    let (base, _) =
        serve(|_| Resp::json(json!({"comments": [{"id": "abc", "author": {}, "body": null}], "total": 1})))
            .await;
    let key = TicketKey::parse("AA-7").unwrap();
    assert!(source(&base).comments(&key).await.is_err());
}

#[tokio::test]
async fn statuses_are_distinct() {
    let (base, log) =
        serve(|_| Resp::json(json!([{"name": "To Do"}, {"name": "Done"}, {"name": "To Do"}, {"id": "x"}])))
            .await;
    let s = source(&base).known_statuses().await.unwrap();
    assert_eq!(s, ["Done", "To Do"]);
    assert_eq!(log.lock().unwrap()[0].path, "/rest/api/3/status");
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let (base, log) = serve(|r| {
        if r.path.starts_with("/rest/api/3/search/jql") {
            let mut x = Resp::status(302);
            x.headers.push(("location", "/elsewhere".into()));
            x
        } else {
            Resp::json(json!({"issues": [], "isLast": true}))
        }
    })
    .await;
    let err = source(&base).search(&filter(), 0).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 302"), "{err}");
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1, "only the first request was made");
    assert!(log.iter().all(|r| r.path != "/elsewhere"));
}

#[tokio::test]
async fn timeout_is_enforced() {
    let (base, _) = serve(|_| {
        let mut r = Resp::json(json!({"issues": [], "isLast": true}));
        r.delay = Duration::from_secs(3);
        r
    })
    .await;
    let s = source_with(&base, Duration::from_secs(1), Duration::from_millis(300), 1 << 20);
    let t = std::time::Instant::now();
    let err = s.search(&filter(), 0).await.unwrap_err().to_string();
    assert!(err.contains("timed out"), "{err}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn body_cap_is_enforced() {
    let (base, _) =
        serve(|_| Resp::json(json!({"issues": [], "isLast": true, "pad": "x".repeat(5000)}))).await;
    let err = source_with(&base, Duration::from_secs(1), Duration::from_secs(5), 1000)
        .search(&filter(), 0)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("too large"), "{err}");
}

#[tokio::test]
async fn basic_auth_header_is_sent_and_correct() {
    let (base, log) = serve(|_| Resp::json(json!({"issues": [], "isLast": true}))).await;
    source(&base).search(&filter(), 0).await.unwrap();
    // base64("ada@example.test:s3cr3t-token-XYZ")
    let expected = "Basic YWRhQGV4YW1wbGUudGVzdDpzM2NyM3QtdG9rZW4tWFla";
    assert_eq!(log.lock().unwrap()[0].auth.as_deref(), Some(expected));
}

#[tokio::test]
async fn mapping_of_tickets() {
    let mut a = issue("AA-1", "2026-01-02T03:04:05.000+0000");
    let mut b = issue("BB-2", "2026-01-02T04:04:05.000+0100");
    a["fields"]["priority"]["name"] = json!("Lowest");
    a["fields"]["status"] = json!({"name": "Done", "statusCategory": {"key": "done"}});
    a["fields"]["resolutiondate"] = json!("2026-01-02T03:04:05.000+0000");
    b["fields"]["status"] = json!({"name": "In Progress", "statusCategory": {"key": "indeterminate"}});
    b["fields"]["priority"]["name"] = json!("Blocker");
    b["fields"]["resolutiondate"] = json!("garbage");
    b["fields"]["url"] = json!("https://evil.test/");
    b["self"] = json!("https://evil.test/");
    let (base, _) =
        serve(move |_| Resp::json(json!({"issues": [a.clone(), b.clone()], "isLast": true}))).await;
    let t = source(&base).search(&filter(), 0).await.unwrap();
    assert_eq!(t[0].priority, -10);
    assert_eq!(t[0].status_category, StatusCategory::Done);
    assert_eq!(t[0].resolved_at, Some(1_767_323_045_000));
    assert_eq!(t[0].url, format!("{base}/browse/AA-1"));
    assert_eq!(t[0].project, "AA");
    assert_eq!(t[0].assignee.as_deref(), Some("Ada"));
    assert_eq!(t[0].description, "hello");
    assert_eq!(t[1].priority, 0, "unknown priority name");
    assert_eq!(t[1].status_category, StatusCategory::InProgress);
    assert_eq!(t[1].resolved_at, None, "unparseable resolution date");
    assert_eq!(
        t[1].url,
        format!("{base}/browse/BB-2"),
        "url never taken from the response"
    );
    // Same instant, different notation: stamps stay verbatim and different.
    assert_eq!(t[0].updated_stamp, "2026-01-02T03:04:05.000+0000");
    assert_eq!(t[1].updated_stamp, "2026-01-02T04:04:05.000+0100");
    assert_ne!(t[0].updated_stamp, t[1].updated_stamp);
}

#[tokio::test]
async fn invalid_issue_key_in_response_fails_closed() {
    let (base, _) = serve(|_| Resp::json(json!({"issues": [issue("aa/../x-1", "u")], "isLast": true}))).await;
    assert!(source(&base).search(&filter(), 0).await.is_err());
}

fn config(vars: &[(&str, &str)]) -> Config {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Config::from_lookup(move |k| {
        if k == "CORVUS_STATE_DIR" {
            return Some("/tmp/corvus-jira-test".into());
        }
        if k == "HOME" {
            return Some("/tmp".into());
        }
        vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    })
    .unwrap()
}

#[test]
fn from_config_none_when_unset_some_when_set() {
    assert!(corvus_jira::from_config(&config(&[])).unwrap().is_none());
    let c = config(&[
        ("CORVUS_JIRA_BASE_URL", "https://acme.atlassian.net/"),
        ("CORVUS_JIRA_EMAIL", EMAIL),
        ("CORVUS_JIRA_API_TOKEN", TOKEN),
        ("CORVUS_JIRA_POOL", "work"),
        ("CORVUS_JIRA_POLL_SECONDS", "30"),
        ("CORVUS_JIRA_IGNORE_ACCOUNTS", "bot1,bot2"),
    ]);
    let s = corvus_jira::from_config(&c).unwrap().unwrap();
    assert_eq!(s.id(), SourceId::Jira);
    assert_eq!(s.site(), "acme.atlassian.net");
    assert_eq!(s.pool(), "work");
    assert_eq!(s.poll_interval(), Duration::from_secs(30));
    assert_eq!(s.ignored_authors(), ["bot1", "bot2"]);
    assert_eq!(s.default_filter().auto_queue[0].projects, ["AA"]);
    assert!(!format!("{s:?}").contains(TOKEN));
}

#[test]
fn plain_http_is_rejected_outside_tests() {
    let mut c = config(&[
        ("CORVUS_JIRA_BASE_URL", "https://acme.atlassian.net"),
        ("CORVUS_JIRA_EMAIL", EMAIL),
        ("CORVUS_JIRA_API_TOKEN", TOKEN),
    ]);
    c.jira.as_mut().unwrap().base_url = "http://acme.atlassian.net".into();
    assert!(corvus_jira::from_config(&c).is_err());
}
