//! Plan 0002: the minimal session/task CLI, through the real binary.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use common::{Home, json_stdout, stderr};
use serde_json::Value;

fn run(home: &Home, args: &[&str]) -> std::process::Output {
    home.corvus().args(args).output().unwrap()
}

fn ok(home: &Home, args: &[&str]) -> Value {
    json_stdout(&run(home, args))
}

#[test]
fn register_create_claim_complete_round_trip() {
    let home = Home::new();
    let s = ok(
        &home,
        &[
            "session",
            "register",
            "--agent",
            "alice",
            "--metadata",
            r#"{"cwd":"/r"}"#,
        ],
    );
    let sid = s["id"].as_str().unwrap().to_owned();
    assert_eq!(s["presence"], "active");
    assert_eq!(s["metadata"]["cwd"], "/r");

    let dep = ok(&home, &["task", "create", "--title", "dep"]);
    assert_eq!(
        dep["creator"], "operator",
        "creator defaults to CORVUS_OPERATOR_NAME"
    );
    let dep_id = dep["id"].as_str().unwrap();
    let child = ok(
        &home,
        &[
            "task",
            "create",
            "--title",
            "child",
            "--priority",
            "9",
            "--depends-on",
            dep_id,
        ],
    );
    assert_eq!(child["blocked"], true);

    let c = ok(&home, &["task", "claim", "--session", &sid]);
    assert_eq!(c["task"]["title"], "dep");
    assert_eq!(c["claim"]["agent_name"], "alice");
    ok(&home, &["task", "renew", dep_id, "--session", &sid]);
    let done = ok(
        &home,
        &["task", "complete", dep_id, "--session", &sid, "--summary", "ok"],
    );
    assert_eq!(done["status"], "done");
    let c2 = ok(&home, &["task", "claim", "--session", &sid]);
    assert_eq!(c2["task"]["title"], "child");
    let child_id = c2["task"]["id"].as_str().unwrap();
    ok(&home, &["task", "release", child_id, "--session", &sid]);

    let none = ok(
        &home,
        &["task", "claim", "--session", &sid, "--pool", "elsewhere"],
    );
    assert_eq!(none, serde_json::json!({ "task": null, "claim": null }));

    let list = ok(&home, &["task", "list", "--status", "open"]);
    assert_eq!(list.as_array().unwrap().len(), 1);
    let got = ok(&home, &["task", "get", child_id]);
    assert_eq!(got["depends_on"][0], dep_id);
    let sessions = ok(&home, &["session", "list"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    ok(&home, &["session", "heartbeat", &sid]);
    let ended = ok(&home, &["session", "end", &sid]);
    assert_eq!(ended["presence"], "ended");
    assert_eq!(
        ok(&home, &["session", "list", "--all"]).as_array().unwrap().len(),
        1
    );
}

#[test]
fn error_exit_codes() {
    let home = Home::new();
    let sid = ok(&home, &["session", "register", "--agent", "a"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = ok(&home, &["session", "register", "--agent", "b"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let t = ok(&home, &["task", "create", "--title", "x"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(&home, &["task", "claim", "--session", &sid]);

    let conflict = run(&home, &["task", "complete", &t, "--session", &other]);
    assert_eq!(conflict.status.code(), Some(5));
    let err: Value = serde_json::from_slice(&conflict.stderr).unwrap();
    assert_eq!(err["error"]["code"], "conflict");

    let gone = run(
        &home,
        &[
            "task",
            "claim",
            "--session",
            "01890000-0000-7000-8000-000000000000",
        ],
    );
    assert_eq!(gone.status.code(), Some(5));
    assert!(stderr(&gone).contains("session_not_active"));

    let missing = run(&home, &["task", "get", "01890000-0000-7000-8000-000000000000"]);
    assert_eq!(missing.status.code(), Some(4));

    let invalid = run(&home, &["task", "create", "--title", " "]);
    assert_eq!(invalid.status.code(), Some(2));
    let bad_status = run(&home, &["task", "list", "--status", "waiting"]);
    assert_eq!(bad_status.status.code(), Some(2));
}

#[test]
fn cancel_and_admin_commands() {
    let home = Home::new();
    let sid = ok(&home, &["session", "register", "--agent", "a"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let t = ok(&home, &["task", "create", "--title", "x"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(&home, &["task", "claim", "--session", &sid]);
    let r = ok(&home, &["task", "cancel", &t, "--reason", "dup", "--cascade"]);
    assert_eq!(r["cancelled"].as_array().unwrap().len(), 1);
    assert_eq!(r["task"]["cancel_reason"], "dup");
    assert_eq!(
        ok(&home, &["admin", "expire-leases"]),
        serde_json::json!({ "expired": 0 })
    );
    assert_eq!(
        ok(&home, &["admin", "end-stale"]),
        serde_json::json!({ "ended": [] })
    );
    let generated = ok(&home, &["session", "register"]);
    assert_eq!(generated["name_generated"], true);
}
