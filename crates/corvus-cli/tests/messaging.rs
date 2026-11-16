//! Plan 0003 through the real binary: a blocking wait in one process woken by a message from another.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Home, json_stdout};
use serde_json::Value;

fn ok(home: &Home, args: &[&str]) -> Value {
    json_stdout(&home.corvus().args(args).output().unwrap())
}

#[test]
fn two_processes_exchange_a_message_with_a_blocking_wait() {
    let home = Home::new();
    ok(&home, &["session", "register", "--agent", "bob"]);
    let started = Instant::now();
    let waiter = home
        .corvus()
        .args(["inbox", "wait", "bob", "--timeout", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500)); // let the waiter subscribe and block
    let sent = ok(&home, &["tell", "bob", "ping from another process"]);
    let sent_at = Instant::now();
    let out = waiter.wait_with_output().unwrap();
    let got = json_stdout(&out);
    assert_eq!(got[0]["id"], sent["id"]);
    assert_eq!(got[0]["body"], "ping from another process");
    assert!(
        sent_at.elapsed() < Duration::from_secs(3),
        "woken by the hint, not the 10 s re-check"
    );
    assert!(started.elapsed() < Duration::from_secs(10));

    // The reply goes back the same way, in the same direct thread.
    let reply = ok(
        &home,
        &["tell", "operator", "pong", "--from", "bob", "--type", "answer"],
    );
    assert_eq!(reply["thread_id"], sent["thread_id"]);
}

#[test]
fn wait_times_out_empty() {
    let home = Home::new();
    let t = Instant::now();
    let got = ok(&home, &["inbox", "wait", "nobody", "--timeout", "1"]);
    assert_eq!(got, serde_json::json!([]));
    assert!(t.elapsed() >= Duration::from_secs(1));
}

#[test]
fn messaging_memory_and_stats_commands() {
    let home = Home::new();
    ok(&home, &["session", "register", "--agent", "bob"]);
    let c = ok(&home, &["connect", "--agent", "carol"]);
    assert_eq!(c["session"]["agent_name"], "carol");
    assert!(c["guidance"].as_str().unwrap().contains("Mail first"));
    let m = ok(&home, &["tell", "bob", "q?", "--type", "question"]);
    let thread = m["thread_id"].as_str().unwrap().to_owned();
    let inbox = ok(&home, &["inbox", "read", "bob"]);
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    let id = m["id"].as_i64().unwrap().to_string();
    assert_eq!(
        ok(&home, &["inbox", "ack", &id, "--agent", "bob"])["newly_acked"],
        true
    );
    assert_eq!(ok(&home, &["inbox", "read", "bob"]), serde_json::json!([]));
    let post = ok(
        &home,
        &[
            "msg",
            "post",
            "--thread",
            &thread,
            "--from",
            "bob",
            "--type",
            "answer",
            "--in-reply-to",
            &id,
            "yes",
        ],
    );
    assert_eq!(post["in_reply_to"].as_i64(), m["id"].as_i64());
    ok(&home, &["digest", "write", "--scope", "global", "all quiet"]);
    assert_eq!(ok(&home, &["digest", "list"])[0]["summary"], "all quiet");
    let d = ok(&home, &["decision", "record", "--title", "Storage", "SQLite"]);
    ok(
        &home,
        &[
            "decision",
            "record",
            "--title",
            "Storage v2",
            "SQLite, WAL",
            "--supersedes",
            d["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(ok(&home, &["decision", "list"]).as_array().unwrap().len(), 1);
    assert_eq!(
        ok(&home, &["decision", "list", "--all"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let b = ok(&home, &["bootstrap", "bob"]);
    assert_eq!(b["decisions"].as_array().unwrap().len(), 1);
    let s = ok(&home, &["stats"]);
    assert_eq!(s["unread_by_agent"]["bob"], 0);
    let e = home.corvus().args(["admin", "export"]).output().unwrap();
    assert_eq!(e.status.code(), Some(2), "no export repo configured");
}

// ---------- `corvus tell --all [--wake]` (plan 0012) ----------

#[test]
fn tell_all_broadcasts_and_wake_marks_it() {
    let home = common::Home::new();
    let run = |args: &[&str]| home.corvus().args(args).output().unwrap();
    for agent in ["alice", "bob"] {
        common::json_stdout(&run(&["session", "register", "--agent", agent]));
    }

    let plain = common::json_stdout(&run(&["tell", "--all", "standup in five"]));
    assert_eq!(plain["recipient"], serde_json::Value::Null);
    assert_eq!(plain["wake"], false);
    assert_eq!(plain["sender"], "operator");

    let woke = common::json_stdout(&run(&["tell", "--all", "--wake", "stop: the build is red"]));
    assert_eq!(woke["wake"], true);
    assert_eq!(woke["thread_id"], plain["thread_id"], "one broadcast thread");

    // Everyone has both in their inbox.
    for agent in ["alice", "bob"] {
        let inbox = common::json_stdout(&run(&["inbox", "read", agent]));
        let bodies: Vec<&str> = inbox
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["body"].as_str().unwrap())
            .collect();
        assert_eq!(bodies, ["standup in five", "stop: the build is red"], "{agent}");
    }

    // A direct message still works, and the argument shapes are checked.
    let direct = common::json_stdout(&run(&["tell", "alice", "just you"]));
    assert_eq!(direct["recipient"], "alice");
    assert_eq!(direct["wake"], false);
    for bad in [
        &["tell", "--all", "alice", "two words"][..],
        &["tell", "only-one-word"][..],
        &["tell", "--wake", "alice", "not a broadcast"][..],
        &[
            "tell",
            "--all",
            "--wake",
            "--from",
            "alice",
            "agents cannot wake everyone",
        ][..],
    ] {
        let out = run(bad);
        assert!(!out.status.success(), "{bad:?} should fail");
    }
}
