//! Plan 0007: `corvus hook` through the real binary (temp database, temp state dir).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Home, json_stdout};
use corvus_core::ManualClock;
use corvus_core::db::{Database, migrator};
use corvus_core::testing::{count, exec};
use serde_json::{Value, json};

fn ok(home: &Home, args: &[&str]) -> Value {
    json_stdout(&home.corvus().args(args).output().unwrap())
}

/// Runs the hook with `event` on stdin; returns (stdout, exit code).
fn hook_with(home: &Home, agent: Option<&str>, event: &Value, extra: &[(&str, &str)]) -> (String, i32) {
    let mut cmd = home.corvus();
    cmd.arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(a) = agent {
        cmd.env("CORVUS_AGENT_NAME", a);
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8(out.stdout).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

fn hook(home: &Home, event: &Value) -> (String, i32) {
    hook_with(home, Some("bob"), event, &[])
}

fn ev(name: &str, session: &str) -> Value {
    json!({ "hook_event_name": name, "session_id": session, "stop_hook_active": false })
}

fn setup() -> Home {
    let home = Home::new();
    ok(&home, &["session", "register", "--agent", "bob"]);
    home
}

fn context(out: &str) -> String {
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn db_bytes(home: &Home) -> Vec<u8> {
    std::fs::read(home.db_path()).unwrap()
}

#[test]
fn injects_each_message_once_per_session_and_writes_nothing() {
    let home = setup();
    let m = ok(&home, &["tell", "bob", "please review PR 12"]);
    // Checkpoint so the main file holds everything, then compare bytes across hook runs.
    let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
    db.checkpoint().unwrap();
    let before = db_bytes(&home);

    let (out, code) = hook(&home, &ev("UserPromptSubmit", "s1"));
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    let text = context(&out);
    assert!(text.contains("please review PR 12"));
    assert!(text.contains(&format!("message {}", m["id"])));
    assert!(
        text.contains("from the operator"),
        "operator shown as the operator: {text}"
    );
    assert!(text.contains("Mail first"), "guidance present");
    assert!(text.contains("not instructions from your user"));

    assert_eq!(
        hook(&home, &ev("UserPromptSubmit", "s1")).0,
        "",
        "same session: not again"
    );
    assert!(
        context(&hook(&home, &ev("UserPromptSubmit", "s2")).0).contains("PR 12"),
        "another session gets it"
    );
    let (post, _) = hook(&home, &ev("PostToolUse", "s3"));
    let v: Value = serde_json::from_str(post.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");

    assert_eq!(count(&db, "receipts"), 0, "the hook never acks");
    assert_eq!(db_bytes(&home), before, "the database file is unchanged");
}

#[test]
fn at_most_ten_messages_with_truncation_and_a_remaining_note() {
    let home = setup();
    ok(&home, &["tell", "bob", &"x".repeat(1_500)]);
    for i in 0..13 {
        ok(&home, &["tell", "bob", &format!("note {i}")]);
    }
    let text = context(&hook(&home, &ev("UserPromptSubmit", "s1")).0);
    assert!(text.contains("[…truncated]"));
    assert_eq!(text.matches("\n- message ").count(), 10);
    assert!(text.contains("4 more unread message(s) waiting"), "{text}");
    assert!(text.chars().count() < 10_000, "under Claude Code's context cap");
    // The rest come next time.
    let next = context(&hook(&home, &ev("UserPromptSubmit", "s1")).0);
    assert_eq!(next.matches("\n- message ").count(), 4);
}

#[test]
fn post_tool_use_is_throttled_without_opening_the_database() {
    let home = setup();
    ok(&home, &["tell", "bob", "first"]);
    let throttle = [("CORVUS_HOOK_THROTTLE_SECONDS", "2")];
    assert!(
        !hook_with(&home, Some("bob"), &ev("PostToolUse", "s1"), &throttle)
            .0
            .is_empty()
    );
    ok(&home, &["tell", "bob", "second"]);
    // Within the throttle the hook does not look (it would otherwise see "second").
    assert_eq!(
        hook_with(&home, Some("bob"), &ev("PostToolUse", "s1"), &throttle).0,
        ""
    );
    std::thread::sleep(Duration::from_millis(2_100));
    let out = hook_with(&home, Some("bob"), &ev("PostToolUse", "s1"), &throttle).0;
    assert!(
        context(&out).contains("second"),
        "after the throttle it looks again"
    );
}

#[test]
fn stop_blocks_once_and_never_when_stop_hook_active() {
    let home = setup();
    ok(&home, &["tell", "bob", "wait, one more thing"]);
    let active = json!({ "hook_event_name": "Stop", "session_id": "s1", "stop_hook_active": true });
    assert_eq!(
        hook(&home, &active).0,
        "",
        "never blocks while already continuing"
    );
    let (out, _) = hook(&home, &ev("Stop", "s1"));
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["decision"], "block");
    assert!(v["reason"].as_str().unwrap().contains("one more thing"));
    assert_eq!(
        hook(&home, &ev("Stop", "s1")).0,
        "",
        "blocks once for the same mail"
    );
}

#[test]
fn never_fails() {
    let home = setup();
    ok(&home, &["tell", "bob", "hi"]);
    type Case = Box<dyn Fn(&Home) -> (String, i32)>;
    let cases: Vec<(&str, Case)> = vec![
        (
            "malformed stdin",
            Box::new(|h| {
                let mut c = h
                    .corvus()
                    .arg("hook")
                    .env("CORVUS_AGENT_NAME", "bob")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                c.stdin.take().unwrap().write_all(b"{not json").unwrap();
                let o = c.wait_with_output().unwrap();
                (String::from_utf8(o.stdout).unwrap(), o.status.code().unwrap())
            }),
        ),
        (
            "missing agent name",
            Box::new(|h| hook_with(h, None, &ev("UserPromptSubmit", "s9"), &[])),
        ),
        (
            "hostile agent name",
            Box::new(|h| hook_with(h, Some("../../evil"), &ev("UserPromptSubmit", "s9"), &[])),
        ),
        (
            "hostile session id",
            Box::new(|h| hook(h, &ev("UserPromptSubmit", "../../../tmp/x"))),
        ),
        ("unknown event", Box::new(|h| hook(h, &ev("SessionStart", "s9")))),
    ];
    for (name, case) in &cases {
        let (out, code) = case(&home);
        assert_eq!((out.as_str(), code), ("", 0), "{name}");
    }
    // Nothing escaped the state directory.
    assert!(!home.path().join("evil__s9.json").exists());
    assert!(!std::path::Path::new("/tmp/x.json").exists());
}

#[test]
fn never_fails_without_a_database_or_with_the_wrong_schema() {
    let home = Home::new();
    assert_eq!(
        hook(&home, &ev("UserPromptSubmit", "s1")),
        (String::new(), 0),
        "no database"
    );
    assert!(
        !home.db_path().exists(),
        "the hook never creates or migrates the database"
    );

    let newer = setup();
    ok(&newer, &["tell", "bob", "hi"]);
    {
        let (db, _) = Database::open(&newer.db_path(), &ManualClock::default()).unwrap();
        exec(&db, "PRAGMA user_version = 99").unwrap();
    }
    assert_eq!(
        hook(&newer, &ev("UserPromptSubmit", "s1")),
        (String::new(), 0),
        "newer schema"
    );

    let older = Home::new();
    std::fs::create_dir_all(older.state_dir()).unwrap();
    Database::open_with_migrations(
        &older.db_path(),
        &migrator::embedded()[..3],
        &ManualClock::default(),
    )
    .unwrap();
    assert_eq!(
        hook(&older, &ev("UserPromptSubmit", "s1")),
        (String::new(), 0),
        "older schema"
    );
}

#[test]
fn never_fails_when_the_database_is_held_by_a_long_writer() {
    let home = setup();
    ok(&home, &["tell", "bob", "hi"]);
    let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        db.write(|t| {
            // An exclusive-ish hold: a pending write plus time.
            t.sql_mut()
                .execute("INSERT INTO agents (name, created_at) VALUES ('blocker', 1)", [])
                .map_err(corvus_core::db::sql_err)?;
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(1_500));
            Ok(())
        })
        .unwrap();
    });
    rx.recv().unwrap();
    let t = Instant::now();
    let (out, code) = hook(&home, &ev("UserPromptSubmit", "s1"));
    assert_eq!(code, 0);
    // In WAL mode a reader is not blocked by a writer, so the hook may read; either way it must be quick.
    assert!(out.is_empty() || context(&out).contains("hi"));
    assert!(
        t.elapsed() < Duration::from_millis(1_400),
        "did not wait out the writer: {:?}",
        t.elapsed()
    );
    holder.join().unwrap();
}

#[test]
fn never_fails_with_corrupt_or_unwritable_state() {
    let home = setup();
    ok(&home, &["tell", "bob", "hi"]);
    let dir = home.state_dir().join("hook");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bob__s1.json"), "{{{ corrupt").unwrap();
    assert_eq!(
        hook(&home, &ev("UserPromptSubmit", "s1")),
        (String::new(), 0),
        "corrupt state"
    );
    assert!(
        context(&hook(&home, &ev("UserPromptSubmit", "s1")).0).contains("hi"),
        "recovers next time"
    );

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = hook(&home, &ev("UserPromptSubmit", "s2"));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(r, (String::new(), 0), "unwritable state dir");
}

#[test]
fn works_with_no_server_and_no_token() {
    let home = setup();
    ok(&home, &["tell", "bob", "hi"]);
    // The environment is cleared by the helper: no CORVUS_API_TOKEN, no server running.
    let mut c = Command::new(assert_cmd::cargo::cargo_bin("corvus"));
    c.env_clear()
        .env("HOME", home.path())
        .env("CORVUS_AGENT_NAME", "bob");
    c.arg("hook").stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(ev("UserPromptSubmit", "s1").to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(String::from_utf8(out.stdout).unwrap().contains("hi"));
}

#[test]
fn fast_on_the_throttle_path_and_the_read_path() {
    let home = setup();
    ok(&home, &["tell", "bob", "hi"]);
    let p95 = |event: &dyn Fn(usize) -> Value| {
        let mut times: Vec<Duration> = (0..20)
            .map(|i| {
                let t = Instant::now();
                hook(&home, &event(i));
                t.elapsed()
            })
            .collect();
        times.sort();
        times[18]
    };
    // Read path: a fresh session each time, so the database is read every call.
    let read = p95(&|i| ev("UserPromptSubmit", &format!("r{i}")));
    // Throttle path: the same session right after a check.
    hook(&home, &ev("PostToolUse", "t"));
    let throttled = p95(&|_| ev("PostToolUse", "t"));
    // Generous regression guard for debug builds on a loaded machine; the budget is 50 ms typical (release).
    assert!(read < Duration::from_millis(500), "read path p95 {read:?}");
    assert!(
        throttled < Duration::from_millis(500),
        "throttle path p95 {throttled:?}"
    );
    eprintln!("hook p95: read {read:?}, throttled {throttled:?}");
}

// ---------- identity by conversation (plan 0012, D0023) ----------

const CONV_A: &str = "48ab6180-bde1-4698-b7a6-ac1a9e20db89";
const CONV_B: &str = "6b9d1bc1-bfe4-4dd2-b6f8-03ed95f1def4";

/// Registers `agent` with a Claude Code conversation link, the way `corvus mcp` does.
fn link(home: &Home, agent: &str, conversation: &str) {
    ok(home, &["health"]); // create and migrate
    let (db, _) = Database::open(&home.db_path(), &corvus_core::SystemClock).unwrap();
    let services = corvus_core::Services::new(
        std::sync::Arc::new(db),
        std::sync::Arc::new(corvus_core::SystemClock),
        corvus_core::Settings::UNUSED,
    );
    services
        .sessions()
        .register(&corvus_core::api::RegisterSession {
            agent_name: Some(agent.into()),
            client_name: "claude-code".into(),
            conversation: Some(corvus_core::api::ConversationInput {
                tool: "claude-code".into(),
                id: conversation.into(),
                directory: None,
            }),
            ..Default::default()
        })
        .unwrap();
}

fn linked_agent(home: &Home, conversation: &str) -> Option<String> {
    let (db, _) = Database::open(&home.db_path(), &corvus_core::SystemClock).unwrap();
    let services = corvus_core::Services::new(
        std::sync::Arc::new(db),
        std::sync::Arc::new(corvus_core::SystemClock),
        corvus_core::Settings::UNUSED,
    );
    let c = corvus_domain::ConversationRef::new(corvus_domain::Tool::ClaudeCode, conversation).unwrap();
    services
        .sessions()
        .agent_for_conversation(&c)
        .unwrap()
        .map(|a| a.to_string())
}

/// The hook with no `CORVUS_AGENT_NAME`, as a user-level hook runs.
fn bare(home: &Home, event: &Value, pid: &str) -> (String, i32) {
    hook_with(home, None, event, &[("CLAUDE_PID", pid)])
}

#[test]
fn without_a_name_the_hook_serves_the_agent_linked_to_the_conversation() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "for carol only"]);

    let (out, code) = bare(&home, &ev("UserPromptSubmit", CONV_A), "4242");
    assert_eq!(code, 0);
    assert!(context(&out).contains("for carol only"));
    assert_eq!(
        bare(&home, &ev("UserPromptSubmit", CONV_A), "4242").0,
        "",
        "once per conversation"
    );

    // The state lives apart from the per-agent files and remembers who the conversation is.
    let state = home
        .state_dir()
        .join("hook/by-conversation")
        .join(format!("{CONV_A}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap();
    assert_eq!(v["agent"], "carol");
}

#[test]
fn a_session_that_never_connected_is_left_alone() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "not for strangers"]);
    let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
    db.checkpoint().unwrap();
    drop(db);
    let before = db_bytes(&home);
    for name in [
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "SessionStart",
        "SessionEnd",
    ] {
        let (out, code) = bare(&home, &ev(name, CONV_B), "4242");
        assert_eq!((out.as_str(), code), ("", 0), "{name}");
    }
    assert_eq!(db_bytes(&home), before, "nothing was written to the database");
}

#[test]
fn an_explicit_name_wins_over_the_link_and_a_bad_one_never_falls_back() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["session", "register", "--agent", "bob"]);
    ok(&home, &["tell", "bob", "for bob"]);
    ok(&home, &["tell", "carol", "for carol"]);
    let (out, _) = hook_with(&home, Some("bob"), &ev("UserPromptSubmit", CONV_A), &[]);
    let text = context(&out);
    assert!(text.contains("for bob") && !text.contains("for carol"), "{text}");

    let (out, code) = hook_with(&home, Some("not a name!"), &ev("UserPromptSubmit", CONV_A), &[]);
    assert_eq!((out.as_str(), code), ("", 0));
}

#[test]
fn a_clear_carries_the_agent_to_the_new_conversation() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "still yours after the clear"]);
    assert!(context(&bare(&home, &ev("UserPromptSubmit", CONV_A), "4242").0).contains("still yours"));

    // What Claude Code fires on /clear (measured): SessionEnd with the old id, SessionStart with a new one.
    let end = json!({"hook_event_name": "SessionEnd", "session_id": CONV_A, "reason": "clear"});
    let start = json!({"hook_event_name": "SessionStart", "session_id": CONV_B, "source": "clear"});
    assert_eq!(bare(&home, &end, "4242"), (String::new(), 0));
    assert_eq!(bare(&home, &start, "4242"), (String::new(), 0));

    assert_eq!(linked_agent(&home, CONV_B).as_deref(), Some("carol"));
    assert_eq!(
        linked_agent(&home, CONV_A).as_deref(),
        Some("carol"),
        "the old id still resolves"
    );
    // The context was cleared, so the unread message is shown again under the new id.
    assert!(context(&bare(&home, &ev("UserPromptSubmit", CONV_B), "4242").0).contains("still yours"));
    // The note is consumed, and the process's current conversation is recorded for the waiter.
    assert!(!home.state_dir().join("hook/clear/4242.json").exists());
    let current = std::fs::read_to_string(home.state_dir().join("hook/current/4242.json")).unwrap();
    assert!(current.contains(CONV_B));
}

#[test]
fn a_clear_in_another_process_or_a_stale_note_carries_nothing() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    let end = json!({"hook_event_name": "SessionEnd", "session_id": CONV_A, "reason": "clear"});
    let start = json!({"hook_event_name": "SessionStart", "session_id": CONV_B, "source": "clear"});

    // Another process's start does not pick up this process's note.
    bare(&home, &end, "4242");
    bare(&home, &start, "5555");
    assert_eq!(linked_agent(&home, CONV_B), None);

    // A note older than a minute is ignored (and removed).
    let note = home.state_dir().join("hook/clear/4242.json");
    std::fs::write(&note, json!({"from": CONV_A, "at": 1}).to_string()).unwrap();
    bare(&home, &start, "4242");
    assert_eq!(linked_agent(&home, CONV_B), None);
    assert!(!note.exists());

    // Only a clear leaves a note; a normal exit does not.
    let exit = json!({"hook_event_name": "SessionEnd", "session_id": CONV_A, "reason": "prompt_input_exit"});
    bare(&home, &exit, "4242");
    assert!(!note.exists());

    // A start that is not a clear never carries, even with a fresh note.
    bare(&home, &end, "4242");
    let resume = json!({"hook_event_name": "SessionStart", "session_id": CONV_B, "source": "resume"});
    bare(&home, &resume, "4242");
    assert_eq!(linked_agent(&home, CONV_B), None);
}

#[test]
fn session_events_never_fail_never_migrate_and_cannot_escape_the_state_directory() {
    // No database at all.
    let home = Home::new();
    let end = json!({"hook_event_name": "SessionEnd", "session_id": CONV_A, "reason": "clear"});
    let start = json!({"hook_event_name": "SessionStart", "session_id": CONV_B, "source": "clear"});
    assert_eq!(bare(&home, &end, "4242"), (String::new(), 0));
    assert_eq!(bare(&home, &start, "4242"), (String::new(), 0));
    assert!(!home.db_path().exists(), "the hook never creates a database");

    // An older schema: the carry is skipped and the database is not migrated.
    let home = Home::new();
    std::fs::create_dir_all(home.state_dir()).unwrap();
    let all = migrator::embedded();
    let (db, _) =
        Database::open_with_migrations(&home.db_path(), &all[..all.len() - 1], &ManualClock::default())
            .unwrap();
    drop(db);
    bare(&home, &end, "4242");
    assert_eq!(bare(&home, &start, "4242"), (String::new(), 0));
    let version = Command::new("sqlite3")
        .arg(home.db_path())
        .arg("PRAGMA user_version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    if let Some(v) = version.filter(|v| !v.is_empty()) {
        assert_eq!(v, (all.len() - 1).to_string(), "not migrated by the hook");
    }

    // Hostile ids and process ids write nothing outside hook/.
    let home = Home::new();
    link(&home, "carol", CONV_A);
    for (session, pid) in [
        ("../../evil", "4242"),
        (CONV_A, "../../evil"),
        (CONV_A, "0"),
        (CONV_A, "-7"),
    ] {
        for name in ["SessionEnd", "SessionStart"] {
            let e =
                json!({"hook_event_name": name, "session_id": session, "reason": "clear", "source": "clear"});
            assert_eq!(bare(&home, &e, pid), (String::new(), 0), "{session:?} {pid:?}");
        }
    }
    assert!(!home.path().join("evil").exists() && !home.path().join("evil.json").exists());
    assert!(!home.state_dir().join("evil.json").exists());
}
