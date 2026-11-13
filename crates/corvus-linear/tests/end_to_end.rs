//! The engine end to end through the real Linear adapter and a fake Linear GraphQL server.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::routing::post;
use corvus_core::Config;
use corvus_core::api::{ListTasks, QueueTicket};
use corvus_core::sources::Sources;
use corvus_core::testing::TestDb;
use corvus_domain::{AutoQueueRule, SourceFilter, SourceId};
use corvus_linear::{Limits, LinearSource};
use serde_json::{Value, json};

fn node(id: &str, team: &str, state: &str, kind: &str, updated: &str) -> Value {
    json!({"identifier": id, "title": format!("Title {id}"), "description": format!("Desc {id}"),
           "url": format!("https://linear.app/acme/issue/{id}"), "priority": 2, "updatedAt": updated,
           "completedAt": null, "state": {"name": state, "type": kind}, "team": {"key": team},
           "assignee": {"name": "Ada"}})
}

fn comment(id: &str, at: &str, body: &str) -> Value {
    json!({"id": id, "body": body, "createdAt": at, "user": {"id": "u9", "name": "Bea"}})
}

#[derive(Default)]
struct Linear {
    issues: Vec<Value>,
    comments: Vec<Value>,
}

#[tokio::test(flavor = "multi_thread")]
async fn assignment_comments_and_manual_queueing() {
    let state = Arc::new(Mutex::new(Linear::default()));
    let s2 = state.clone();
    let app = axum::Router::new().route(
        "/graphql",
        post(move |Json(req): Json<Value>| {
            let s = s2.clone();
            async move {
                let q = req["query"].as_str().unwrap_or_default();
                let st = s.lock().unwrap();
                let body = if q.contains("workflowStates") {
                    json!({"data": {"workflowStates": {"nodes": [{"name": "Todo"}]}}})
                } else if q.contains("comments") {
                    json!({"data": {"issue": {"comments": {"nodes": st.comments,
                        "pageInfo": {"hasNextPage": false, "endCursor": null}}}}})
                } else {
                    json!({"data": {"issues": {"nodes": st.issues,
                        "pageInfo": {"hasNextPage": false, "endCursor": null}}}})
                };
                Json(body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config = Config::from_lookup(|k| match k {
        "HOME" => Some("/home/u".into()),
        "CORVUS_LINEAR_API_KEY" => Some("lin_api_test".into()),
        _ => None,
    })
    .unwrap();
    let src =
        LinearSource::with_endpoint_for_tests(&endpoint, config.linear.as_ref().unwrap(), Limits::default())
            .unwrap();
    let t = TestDb::new();
    let sources = Sources::new(t.services(), t.dir.path().join("state"), vec![Arc::new(src)]);

    // Linear's default has no rules: nothing auto-queues until the operator adds one.
    state.lock().unwrap().issues = vec![
        node("ENG-1", "ENG", "Todo", "unstarted", "2026-01-01T00:00:00.000Z"),
        node("OPS-2", "OPS", "Todo", "unstarted", "2026-01-01T00:00:00.000Z"),
    ];
    state.lock().unwrap().comments = vec![comment("c1", "2026-01-01T00:00:01.000Z", "hello from Linear")];
    let r = sources.sync(SourceId::Linear).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.stats.unwrap().tasks_created, 0);

    // Add a rule for ENG → it queues with the Linear priority map (2 → 5).
    let mut f: SourceFilter = corvus_domain::linear::default_filter();
    f.auto_queue = vec![AutoQueueRule {
        projects: vec!["ENG".into()],
        statuses: vec!["Todo".into()],
    }];
    let view = sources.save_filter(SourceId::Linear, f).await.unwrap();
    assert_eq!(view.known_statuses, vec!["Todo"]);
    let tasks = t.services().huginn().list_tasks(&ListTasks::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        (
            tasks[0].title.as_str(),
            tasks[0].priority,
            tasks[0].creator.as_str()
        ),
        ("[ENG-1] Title ENG-1", 5, "linear")
    );
    assert!(
        tasks[0].body.contains("UNTRUSTED text copied from Linear")
            && tasks[0].body.contains("hello from Linear")
    );

    // A new comment (same millisecond as nothing before; later createdAt) is appended to the open task.
    {
        let mut s = state.lock().unwrap();
        s.issues[0] = node("ENG-1", "ENG", "Todo", "unstarted", "2026-01-01T00:05:00.000Z");
        s.comments
            .push(comment("c2", "2026-01-01T00:04:00.000Z", "a follow-up note"));
    }
    let r = sources.sync(SourceId::Linear).await.unwrap();
    assert_eq!(r.stats.unwrap().comments_delivered, 1);
    let eng = t.services().huginn().get_task(&tasks[0].id).unwrap();
    assert!(eng.body.contains("a follow-up note"));

    // OPS-2 is on the board without a rule: queue it by hand.
    let manual = sources
        .queue(SourceId::Linear, &QueueTicket { key: "OPS-2".into() })
        .await
        .unwrap();
    assert!(manual.title.starts_with("[OPS-2]"));
    let view = sources.view(SourceId::Linear).await.unwrap();
    let ops = view.tickets.iter().find(|t| t.key == "OPS-2").unwrap();
    assert!(ops.manual && ops.url.starts_with("https://linear.app/"));
}
