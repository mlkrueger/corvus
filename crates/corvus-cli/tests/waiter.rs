//! Plan 0012: `corvus hook wait` through the real binary (temp database, temp state dir).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{Home, json_stdout};
use corvus_core::db::{Database, migrator};
use corvus_core::{ManualClock, Services, Settings, SystemClock};
use serde_json::{Value, json};

const CONV_A: &str = "48ab6180-bde1-4698-b7a6-ac1a9e20db89";
const CONV_B: &str = "6b9d1bc1-bfe4-4dd2-b6f8-03ed95f1def4";

fn ok(home: &Home, args: &[&str]) -> Value {
    json_stdout(&home.corvus().args(args).output().unwrap())
}

fn services(home: &Home) -> Services {
    let (db, _) = Database::open(&home.db_path(), &SystemClock).unwrap();
    Services::new(
        std::sync::Arc::new(db),
        std::sync::Arc::new(SystemClock),
        Settings::UNUSED,
    )
}

/// Registers `agent` with a Claude Code conversation link, the way `corvus mcp` does.
fn link(home: &Home, agent: &str, conversation: &str) {
    ok(home, &["health"]);
    services(home)
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

fn waiter_cmd(home: &Home, agent: Option<&str>, pid: &str, extra: &[(&str, &str)]) -> Command {
    let mut cmd = home.corvus();
    cmd.args(["hook", "wait"])
        .env("CLAUDE_PID", pid)
        .env("CLAUDE_CODE_ENTRYPOINT", "cli")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(a) = agent {
        cmd.env("CORVUS_AGENT_NAME", a);
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd
}

/// Starts a waiter for `conversation` in the `claude` process `pid`.
fn start(home: &Home, agent: Option<&str>, conversation: &str, pid: &str, extra: &[(&str, &str)]) -> Child {
    let mut child = waiter_cmd(home, agent, pid, extra).spawn().unwrap();
    let event = json!({"hook_event_name": "Stop", "session_id": conversation, "stop_hook_active": false});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    child
}

/// Waits up to `within` for the waiter to exit. Returns `(exit code, stdout, stderr)`.
fn finish(mut child: Child, within: Duration) -> Option<(i32, String, String)> {
    let end = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let (mut out, mut err) = (String::new(), String::new());
            child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
            child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
            return Some((status.code().unwrap_or(-1), out, err));
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn still_running(child: &mut Child, after: Duration) -> bool {
    std::thread::sleep(after);
    child.try_wait().unwrap().is_none()
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn hook(home: &Home, agent: Option<&str>, event: &Value, pid: &str) -> String {
    let mut cmd = home.corvus();
    cmd.arg("hook")
        .env("CLAUDE_PID", pid)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(a) = agent {
        cmd.env("CORVUS_AGENT_NAME", a);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
}

fn unread(home: &Home, agent: &str) -> usize {
    ok(home, &["inbox", "read", agent]).as_array().unwrap().len()
}

#[test]
fn blocks_until_direct_mail_then_exits_2_with_it_on_stderr_and_acks_nothing() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    let mut w = start(&home, None, CONV_A, "4242", &[]);
    assert!(
        still_running(&mut w, Duration::from_millis(900)),
        "nothing to wake for yet"
    );

    ok(&home, &["tell", "carol", "the build is red"]);
    let (code, out, err) = finish(w, Duration::from_secs(5)).expect("woke within 5 s");
    assert_eq!(code, 2, "exit 2 is the wake signal");
    assert_eq!(out, "", "stdout stays empty");
    assert!(err.contains("the build is red"), "{err}");
    assert!(err.contains("Mail first"), "the guidance comes with it: {err}");
    assert!(err.contains("from the operator"));
    assert_eq!(unread(&home, "carol"), 1, "the waiter never acks");

    // The wake arrives as a prompt; the inbox hook must not show the same message again.
    let event = json!({"hook_event_name": "UserPromptSubmit", "session_id": CONV_A});
    assert_eq!(hook(&home, None, &event, "4242"), "");
}

#[test]
fn mail_already_shown_does_not_wake_again_and_new_mail_does() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "alpha-message"]);
    // Shown by the inbox hook at a prompt; still unread.
    let event = json!({"hook_event_name": "UserPromptSubmit", "session_id": CONV_A});
    assert!(hook(&home, None, &event, "4242").contains("alpha-message"));

    // Ten stops in a row: no wake loop.
    for _ in 0..10 {
        let mut w = start(&home, None, CONV_A, "4242", &[]);
        assert!(
            still_running(&mut w, Duration::from_millis(350)),
            "shown mail must not wake"
        );
        stop(w);
    }

    let w = start(&home, None, CONV_A, "4242", &[]);
    std::thread::sleep(Duration::from_millis(300));
    ok(&home, &["tell", "carol", "beta-message"]);
    let (code, _, err) = finish(w, Duration::from_secs(5)).expect("woke");
    assert_eq!(code, 2);
    assert!(
        err.contains("beta-message") && !err.contains("alpha-message"),
        "{err}"
    );
}

#[test]
fn an_ordinary_broadcast_waits_and_a_waking_one_wakes_with_both() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    let mut w = start(&home, None, CONV_A, "4242", &[]);
    ok(&home, &["tell", "--all", "standup notes are up"]);
    assert!(
        still_running(&mut w, Duration::from_millis(1200)),
        "an ordinary broadcast does not wake"
    );

    ok(&home, &["tell", "--all", "--wake", "stop: the build is red"]);
    let (code, _, err) = finish(w, Duration::from_secs(5)).expect("woke");
    assert_eq!(code, 2);
    assert!(err.contains("stop: the build is red"), "{err}");
    assert!(
        err.contains("standup notes are up"),
        "waking anyway, it shows what is new: {err}"
    );
}

#[test]
fn one_waiter_per_claude_process() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    link(&home, "dave", CONV_B);
    let first = start(&home, None, CONV_A, "4242", &[]);
    std::thread::sleep(Duration::from_millis(300));

    // Every stop starts another: it finds the lock held and leaves at once.
    let second = start(&home, None, CONV_A, "4242", &[]);
    let (code, out, err) = finish(second, Duration::from_secs(3)).expect("the second exits");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));

    // Another claude process has its own waiter; each wakes only for its own agent.
    let mut other = start(&home, None, CONV_B, "5555", &[]);
    ok(&home, &["tell", "carol", "for carol"]);
    let (code, _, err) = finish(first, Duration::from_secs(5)).expect("carol's waiter woke");
    assert_eq!(code, 2);
    assert!(err.contains("for carol"));
    assert!(
        still_running(&mut other, Duration::from_millis(700)),
        "dave's session stays asleep"
    );
    stop(other);
}

#[test]
fn stops_quietly_on_sigterm_and_when_its_session_is_gone() {
    let home = Home::new();
    link(&home, "carol", CONV_A);

    // Claude Code ends its hooks with SIGTERM (normal exit, or the hook's time limit).
    let w = start(&home, None, CONV_A, "4242", &[]);
    std::thread::sleep(Duration::from_millis(500));
    Command::new("kill").arg(w.id().to_string()).status().unwrap();
    let (code, out, err) = finish(w, Duration::from_secs(3)).expect("exits on SIGTERM");
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));

    // A killed session orphans its hooks: the waiter notices its parent is gone and leaves.
    let pid_file = home.path().join("waiter.pid");
    let script = format!(
        "printf '%s' '{{\"session_id\":\"{CONV_A}\"}}' | \"$CORVUS_BIN\" hook wait & echo $! > '{}'; sleep 0.4",
        pid_file.display()
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .env("CLAUDE_PID", "6001")
        .env("CLAUDE_CODE_ENTRYPOINT", "cli")
        .env("CORVUS_BIN", assert_cmd::cargo::cargo_bin("corvus"))
        .status()
        .unwrap();
    assert!(status.success());
    let waiter_pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    let alive = || {
        Command::new("kill")
            .args(["-0", &waiter_pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let end = Instant::now() + Duration::from_secs(4);
    while alive() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(), "the orphaned waiter exited on its own");
}

#[test]
fn never_runs_headless_and_never_fails() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "waiting for you"]);

    // `claude -p` never exits while a waiter runs, so the waiter does not run there — mail or not.
    for entry in ["sdk-cli", "sdk-ts", "anything-else"] {
        let w = start(&home, None, CONV_A, "4242", &[("CLAUDE_CODE_ENTRYPOINT", entry)]);
        let (code, out, err) = finish(w, Duration::from_secs(3)).expect("exits at once");
        assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "{entry}");
    }

    // Bad input and bad names: silent exit 0.
    for (agent, stdin) in [
        (None, "not json".to_owned()),
        (None, json!({"session_id": "../../evil"}).to_string()),
        (None, String::new()),
        (Some("not a name!"), json!({"session_id": CONV_A}).to_string()),
    ] {
        let mut child = waiter_cmd(&home, agent, "4242", &[]).spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let (code, out, err) = finish(child, Duration::from_secs(3)).expect("exits at once");
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (0, "", ""),
            "{agent:?} {stdin:?}"
        );
    }

    // No database, and a database at another schema version: silent exit 0, nothing created or migrated.
    let empty = Home::new();
    let w = start(&empty, None, CONV_A, "4242", &[]);
    assert_eq!(finish(w, Duration::from_secs(3)).expect("exits").0, 0);
    assert!(!empty.db_path().exists());

    let old = Home::new();
    std::fs::create_dir_all(old.state_dir()).unwrap();
    let all = migrator::embedded();
    drop(
        Database::open_with_migrations(&old.db_path(), &all[..all.len() - 1], &ManualClock::default())
            .unwrap(),
    );
    let w = start(&old, Some("carol"), CONV_A, "4242", &[]);
    assert_eq!(finish(w, Duration::from_secs(3)).expect("exits").0, 0);
}

#[test]
fn a_session_that_registers_later_becomes_wakeable() {
    let home = Home::new();
    ok(&home, &["health"]);
    // Armed at SessionStart, before the model has registered with Corvus.
    let mut w = start(&home, None, CONV_A, "4242", &[]);
    assert!(
        still_running(&mut w, Duration::from_millis(600)),
        "not linked yet: it keeps waiting"
    );

    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "welcome"]);
    let (code, _, err) = finish(w, Duration::from_secs(9)).expect("woke after the link appeared");
    assert_eq!(code, 2);
    assert!(err.contains("welcome"));
}

#[test]
fn follows_the_session_across_a_clear() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    let w = start(&home, None, CONV_A, "4242", &[]);
    std::thread::sleep(Duration::from_millis(400));

    // /clear in the same claude process: the agent is carried to the new id and the waiter follows it.
    let end = json!({"hook_event_name": "SessionEnd", "session_id": CONV_A, "reason": "clear"});
    let begin = json!({"hook_event_name": "SessionStart", "session_id": CONV_B, "source": "clear"});
    hook(&home, None, &end, "4242");
    hook(&home, None, &begin, "4242");

    ok(&home, &["tell", "carol", "after the clear"]);
    let (code, _, err) = finish(w, Duration::from_secs(5)).expect("woke");
    assert_eq!(code, 2);
    assert!(err.contains("after the clear"));
    // Shown under the new conversation, so the prompt that follows the wake does not repeat it.
    let prompt = json!({"hook_event_name": "UserPromptSubmit", "session_id": CONV_B});
    assert_eq!(hook(&home, None, &prompt, "4242"), "");
}

#[test]
fn an_explicit_agent_name_works_without_a_link() {
    let home = Home::new();
    ok(&home, &["session", "register", "--agent", "bob"]);
    let w = start(&home, Some("bob"), "plain-session-1", "4242", &[]);
    std::thread::sleep(Duration::from_millis(300));
    ok(&home, &["tell", "bob", "per-project setup still works"]);
    let (code, _, err) = finish(w, Duration::from_secs(5)).expect("woke");
    assert_eq!(code, 2);
    assert!(err.contains("per-project setup still works"));
}

#[test]
fn a_database_held_by_a_long_writer_never_makes_it_fail() {
    let home = Home::new();
    link(&home, "carol", CONV_A);
    ok(&home, &["tell", "carol", "held-message"]);
    let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        db.write(|t| {
            t.sql_mut()
                .execute("INSERT INTO agents (name, created_at) VALUES ('blocker', 1)", [])
                .map_err(corvus_core::db::sql_err)?;
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(2_000));
            Ok(())
        })
        .unwrap();
    });
    rx.recv().unwrap();
    // The waiter only reads (WAL): it wakes with the mail, or at worst leaves silently — never an error.
    let w = start(&home, None, CONV_A, "4242", &[]);
    let (code, out, err) = finish(w, Duration::from_secs(8)).expect("exits");
    holder.join().unwrap();
    assert_eq!(out, "");
    match code {
        2 => assert!(err.contains("held-message"), "{err}"),
        0 => assert_eq!(err, ""),
        other => panic!("exit {other}: {err}"),
    }
}
