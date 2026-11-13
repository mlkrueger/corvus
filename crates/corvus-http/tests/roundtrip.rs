#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{env, get, post, register};
use serde_json::{Value, json};

fn ok(r: &common::Reply) -> Value {
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.body));
    r.json()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[tokio::test]
async fn sessions_and_connect() {
    let e = env();
    let reg = ok(&post(
        &e.app,
        "/api/sessions",
        &json!({"agent_name": "alice", "client_name": "t"}),
    )
    .await);
    let sid = s(&reg["id"]).to_owned();
    assert_eq!(reg["agent_name"], "alice");

    let hb = ok(&post(&e.app, &format!("/api/sessions/{sid}/heartbeat"), &json!({})).await);
    assert_eq!(hb["id"], sid.as_str());

    let list = ok(&get(&e.app, "/api/sessions").await);
    assert_eq!(list.as_array().unwrap().len(), 1);
    let by_agent = ok(&get(&e.app, "/api/sessions?agent=nobody").await);
    assert!(by_agent.as_array().unwrap().is_empty());

    let ended = ok(&post(&e.app, &format!("/api/sessions/{sid}/end"), &json!({})).await);
    assert!(ended["ended_at"].is_string());
    assert!(
        ok(&get(&e.app, "/api/sessions").await)
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(&get(&e.app, "/api/sessions?include_ended=true").await)
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let c = ok(&post(
        &e.app,
        "/api/connect",
        &json!({"agent_name": "bob", "client_name": "t"}),
    )
    .await);
    assert_eq!(c["session"]["agent_name"], "bob");
    assert!(c["guidance"].is_string());
    assert!(c["bootstrap"].is_object());
}

#[tokio::test]
async fn tasks_lifecycle() {
    let e = env();
    let sid = register(&e.app, "alice").await;

    // Nothing to claim yet: 200 with nulls.
    let none = ok(&post(&e.app, "/api/tasks/claim", &json!({"session_id": sid})).await);
    assert_eq!(none, json!({"task": null, "claim": null}));

    let t = ok(&post(
        &e.app,
        "/api/tasks",
        &json!({"creator": "alice", "title": "one", "body": "do it"}),
    )
    .await);
    let tid = s(&t["id"]).to_owned();
    assert_eq!(t["status"], "open");

    assert_eq!(ok(&get(&e.app, "/api/tasks").await).as_array().unwrap().len(), 1);
    assert_eq!(
        ok(&get(&e.app, "/api/tasks?status=open&limit=5").await)
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        ok(&get(&e.app, "/api/tasks?status=done").await)
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(&get(&e.app, &format!("/api/tasks/{tid}")).await)["title"],
        "one"
    );

    let claimed = ok(&post(&e.app, "/api/tasks/claim", &json!({"session_id": sid})).await);
    assert_eq!(claimed["task"]["id"], tid.as_str());
    assert!(claimed["claim"]["lease_expires_at"].is_string());

    let renewed = ok(&post(
        &e.app,
        &format!("/api/tasks/{tid}/renew"),
        &json!({"session_id": sid}),
    )
    .await);
    assert_eq!(renewed["status"], "claimed");

    let done = ok(&post(
        &e.app,
        &format!("/api/tasks/{tid}/complete"),
        &json!({"session_id": sid, "result_summary": "ok"}),
    )
    .await);
    assert_eq!(done["status"], "done");
    assert_eq!(done["result_summary"], "ok");

    // release: claim then give back.
    let t2 = ok(&post(&e.app, "/api/tasks", &json!({"creator": "alice", "title": "two"})).await);
    let id2 = s(&t2["id"]).to_owned();
    ok(&post(&e.app, "/api/tasks/claim", &json!({"session_id": sid})).await);
    let released = ok(&post(
        &e.app,
        &format!("/api/tasks/{id2}/release"),
        &json!({"session_id": sid}),
    )
    .await);
    assert_eq!(released["status"], "open");

    // cancel
    let cancelled = ok(&post(
        &e.app,
        &format!("/api/tasks/{id2}/cancel"),
        &json!({"by": "alice", "reason": "nope", "cascade": true}),
    )
    .await);
    assert_eq!(cancelled["task"]["status"], "cancelled");
    assert_eq!(cancelled["cancelled"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn threads_messages_inbox() {
    let e = env();
    let alice = register(&e.app, "alice").await;
    register(&e.app, "bob").await;

    let th = ok(&post(
        &e.app,
        "/api/threads",
        &json!({"creator": "alice", "title": "talk"}),
    )
    .await);
    let thid = s(&th["id"]).to_owned();
    assert_eq!(
        ok(&get(&e.app, &format!("/api/threads/{thid}")).await)["title"],
        "talk"
    );

    let m = ok(&post(
        &e.app,
        "/api/messages",
        &json!({"thread_id": thid, "from": "alice", "to": "bob", "body": "hello", "session_id": alice}),
    )
    .await);
    let mid = m["id"].as_i64().unwrap();
    assert_eq!(m["type"], "note");

    let in_thread = ok(&get(
        &e.app,
        &format!("/api/threads/{thid}/messages?limit=10&after_id=0"),
    )
    .await);
    assert_eq!(in_thread.as_array().unwrap().len(), 1);

    let listed = ok(&get(&e.app, "/api/messages?limit=10&agent=bob&unread_only=true").await);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["thread_title"], "talk");

    let d = ok(&post(
        &e.app,
        "/api/messages/direct",
        &json!({"from": "alice", "to": "bob", "body": "psst"}),
    )
    .await);
    assert_eq!(d["recipient"], "bob");

    let inbox = ok(&get(&e.app, "/api/inbox/bob?limit=10").await);
    assert_eq!(inbox.as_array().unwrap().len(), 2);
    assert!(
        ok(&get(&e.app, "/api/inbox/alice").await)
            .as_array()
            .unwrap()
            .is_empty()
    );

    let ack = ok(&post(
        &e.app,
        &format!("/api/messages/{mid}/ack"),
        &json!({"agent": "bob"}),
    )
    .await);
    assert_eq!(ack, json!({"message_id": mid, "newly_acked": true}));
    assert_eq!(
        ok(&get(&e.app, "/api/inbox/bob").await).as_array().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn digests_decisions_bootstrap_events_stats_overview_health() {
    let e = env();
    register(&e.app, "alice").await;

    let none = get(&e.app, "/api/digests/latest?scope=global").await;
    assert_eq!(ok(&none), Value::Null);
    let d = ok(&post(
        &e.app,
        "/api/digests",
        &json!({"author": "alice", "scope": "global", "summary": "state"}),
    )
    .await);
    assert_eq!(d["scope"], "global");
    assert_eq!(
        ok(&get(&e.app, "/api/digests/latest?scope=global").await)["summary"],
        "state"
    );
    let a = ok(&post(
        &e.app,
        "/api/digests",
        &json!({"author": "alice", "scope": "agent", "ref": "alice", "summary": "me"}),
    )
    .await);
    assert_eq!(a["ref"], "alice");
    assert_eq!(
        ok(&get(&e.app, "/api/digests/latest?scope=agent&ref=alice").await)["summary"],
        "me"
    );
    assert_eq!(
        ok(&get(&e.app, "/api/digests").await).as_array().unwrap().len(),
        2
    );

    let dec = ok(&post(
        &e.app,
        "/api/decisions",
        &json!({"decided_by": "alice", "title": "use x", "decision": "x", "rationale": "because"}),
    )
    .await);
    assert_eq!(dec["title"], "use x");
    let listed = ok(&get(&e.app, "/api/decisions?include_superseded=true&limit=10").await);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(
        ok(&get(&e.app, "/api/decisions?task=0190f2a0-0000-7000-8000-000000000000").await)
            .as_array()
            .is_none_or(|a| a.is_empty())
    );

    let b = ok(&get(&e.app, "/api/bootstrap/alice").await);
    assert_eq!(b["agent"], "alice");
    assert_eq!(b["decisions"].as_array().unwrap().len(), 1);

    let ev = ok(&get(&e.app, "/api/events?limit=3").await);
    assert!(!ev.as_array().unwrap().is_empty() && ev.as_array().unwrap().len() <= 3);
    let before = ev.as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
    let older = ok(&get(&e.app, &format!("/api/events?before_id={before}")).await);
    assert!(
        older
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["id"].as_i64().unwrap() < before)
    );

    assert!(ok(&get(&e.app, "/api/stats").await)["open_tasks"].is_number());
    assert!(ok(&get(&e.app, "/api/overview").await)["tasks"]["open"].is_array());
    let h = ok(&get(&e.app, "/api/health").await);
    assert_eq!(h["ok"], true);
}

#[tokio::test]
async fn admin_routes() {
    let e = env();
    register(&e.app, "alice").await;
    assert_eq!(
        ok(&post(&e.app, "/api/admin/expire-leases", &json!({})).await),
        json!({"expired": 0})
    );
    assert_eq!(
        ok(&post(&e.app, "/api/admin/end-stale-sessions", &json!({})).await),
        json!({"ended": []})
    );
    // No export repository is configured on the test services.
    let r = post(&e.app, "/api/admin/export", &json!({})).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.code(), "validation");
    // An empty body is the all-defaults request; same answer.
    let r = common::send(&e.app, common::Req::post("/api/admin/export").token()).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let r = post(&e.app, "/api/admin/export", &json!({"nope": 1})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
}
