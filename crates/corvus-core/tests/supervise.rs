//! Plan 0012 step 10: the background leader wakes agents bound from a conversation.
//!
//! Real SQLite file, real stub commands, real short intervals (the code spawns processes and takes file
//! locks). The stub stands in for `opencode` through a user adapter file of that name.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use corvus_core::Corvus;
use corvus_core::api::{ConversationInput, CreateThread, PostMessage, RegisterSession, SendDirect};
use corvus_core::background::{LoopTimings, STEPS, run_background_with_sources};
use corvus_core::delivery::supervise::{SuperviseTimings, WakeSupervision, supervise, targets};
use corvus_core::delivery::watch::{WatchLock, WatchTimings};
use corvus_core::delivery::{adapters, bindings};
use corvus_core::hermod::{Hermod, HermodConfig};
use corvus_core::testing::TestDb;
use corvus_domain::{AgentName, MessageType, WAKE_NOTE};
use tokio::sync::watch;

const SES_A: &str = "ses_e1adfefedffeejMuX56NOipUZx";
const SES_B: &str = "ses_e1ae272a6ffel20y2qkVLnxWQO";
const CLAUDE: &str = "0d4c7f0e-51a5-4b0e-9a55-0d2f6c1b7a10";

/// Appends its arguments (one line per run, tab-separated) to `$1`.
const RECORDER: &str = r#"#!/bin/sh
out="$1"; shift
( IFS='	'; printf '%s\n' "$*" ) >> "$out"
"#;

fn fast() -> SuperviseTimings {
    SuperviseTimings {
        reconcile: Duration::from_millis(50),
        watch: WatchTimings {
            recheck: Duration::from_millis(100),
            backoff_base: Duration::from_millis(100),
            backoff_cap: Duration::from_millis(300),
        },
    }
}

struct Env {
    t: TestDb,
    corvus: Corvus,
    state: PathBuf,
    runs: PathBuf,
}

/// A database, a state directory and a user adapter named `opencode` that runs the recorder.
fn env() -> Env {
    let t = TestDb::new();
    let hermod = Hermod::with_config(
        &t.path,
        HermodConfig {
            poll: Duration::from_millis(20),
            recheck: Duration::from_millis(100),
            detect: true,
        },
    );
    let corvus = Corvus::new(t.services(), hermod);
    let state = t.dir.path().join("state");
    let tools = t.dir.path().join("tools");
    std::fs::create_dir_all(state.join("adapters")).unwrap();
    std::fs::create_dir_all(&tools).unwrap();
    let stub = tools.join("recorder");
    std::fs::write(&stub, RECORDER).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runs = tools.join("runs");
    std::fs::write(
        state.join("adapters/opencode.toml"),
        format!(
            "name = \"opencode\"\n[binding]\nconversation = {{ required = true, pattern = \"chars:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_:8-64\" }}\n\
             [wake]\ncommand = [{:?}, {:?}, \"{{agent}}\", \"{{conversation}}\", \"{{note}}\"]\ntimeout_seconds = 5\n",
            stub.display().to_string(),
            runs.display().to_string()
        ),
    )
    .unwrap();
    Env {
        t,
        corvus,
        state,
        runs,
    }
}

impl Env {
    /// Registers a session for a tool conversation; returns the agent's name.
    fn session(&self, name: &str, tool: &str, id: &str) -> String {
        self.corvus
            .services()
            .sessions()
            .register(&RegisterSession {
                agent_name: Some(name.into()),
                client_name: "test".into(),
                conversation: Some(ConversationInput {
                    tool: tool.into(),
                    id: id.into(),
                    directory: None,
                }),
                ..RegisterSession::default()
            })
            .unwrap()
            .agent_name
    }

    fn tell(&self, to: &str, body: &str) -> i64 {
        self.t.agent("operator");
        self.corvus
            .services()
            .muninn()
            .send_direct(&SendDirect {
                from: "operator".into(),
                to: to.into(),
                body: body.into(),
                kind: MessageType::Note,
                idempotency_key: None,
                session_id: None,
            })
            .unwrap()
            .id
    }

    fn broadcast(&self, body: &str, wake: bool) {
        self.t.agent("operator");
        let m = self.corvus.services().muninn();
        let thread = m
            .create_thread(&CreateThread {
                creator: "operator".into(),
                title: "Operator broadcasts".into(),
                task_id: None,
                key: Some(corvus_core::api::BROADCAST_THREAD_KEY.to_owned()),
                session_id: None,
            })
            .unwrap();
        m.post_message(&PostMessage {
            thread_id: thread.id,
            from: "operator".into(),
            body: body.into(),
            kind: MessageType::Note,
            to: None,
            in_reply_to: None,
            artifacts: Vec::new(),
            idempotency_key: None,
            session_id: None,
            wake,
        })
        .unwrap();
    }

    /// The recorder's runs: `[agent, conversation, note]` each.
    fn runs(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(&self.runs)
            .unwrap_or_default()
            .lines()
            .map(|l| l.split('\t').map(str::to_owned).collect())
            .collect()
    }

    fn start(&self) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
        let (stop, rx) = watch::channel(false);
        let h = tokio::spawn(supervise(self.corvus.clone(), self.state.clone(), fast(), rx));
        (stop, h)
    }
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn stop(stop: watch::Sender<bool>, h: tokio::task::JoinHandle<()>) {
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .expect("supervisor stops")
        .unwrap();
}

#[test]
fn the_built_in_opencode_adapter_takes_a_session_id_and_only_the_fixed_note() {
    let dir = tempfile::tempdir().unwrap();
    let a = adapters::load(dir.path(), "opencode").unwrap().adapter;
    a.validate_binding(Some(SES_A)).unwrap();
    assert!(a.validate_binding(Some("ses; rm -rf ~")).is_err());
    assert!(a.validate_binding(None).is_err());
    let agent = AgentName::parse("bob").unwrap();
    assert_eq!(
        a.render_wake(&agent, Some(SES_A), 1, "/state"),
        ["opencode", "run", "--session", SES_A, WAKE_NOTE]
    );
}

#[test]
fn targets_are_agents_whose_newest_conversation_has_an_adapter_and_no_explicit_binding() {
    let e = env();
    let bob = e.session("bob", "opencode", SES_A);
    let carol = e.session("carol", "claude-code", CLAUDE);
    let dave = e.session("dave", "opencode", SES_B);
    let got = targets(&e.corvus, &e.state).unwrap();
    let names: Vec<&str> = got.keys().map(AgentName::as_str).collect();
    assert_eq!(
        names,
        [bob.as_str(), dave.as_str()],
        "claude-code has no adapter: {carol}"
    );
    assert_eq!(got[&AgentName::parse(&bob).unwrap()].conversation, SES_A);

    // An explicit binding wins: that agent is left to `corvus watch` / `corvus up`.
    let adapter = adapters::load(&e.state, "opencode").unwrap().adapter;
    bindings::bind(&e.state, &dave, &adapter, Some(SES_B), false).unwrap();
    let got = targets(&e.corvus, &e.state).unwrap();
    assert_eq!(
        got.keys().map(AgentName::as_str).collect::<Vec<_>>(),
        [bob.as_str()]
    );
}

#[test]
fn a_malformed_bindings_file_is_an_error_not_an_empty_set() {
    let e = env();
    e.session("bob", "opencode", SES_A);
    std::fs::create_dir_all(e.state.join("agents")).unwrap();
    std::fs::write(e.state.join("agents/bindings.json"), "{not json").unwrap();
    assert!(targets(&e.corvus, &e.state).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_mail_wakes_the_conversation_once_with_only_the_fixed_note() {
    let e = env();
    let bob = e.session("bob", "opencode", SES_A);
    let (tx, h) = e.start();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(e.runs().is_empty(), "no mail, no wake");

    e.tell(&bob, "SECRET-BODY please look");
    eventually("woken", || e.runs().len() == 1).await;
    assert_eq!(e.runs()[0], [bob.as_str(), SES_A, WAKE_NOTE]);
    // Unread but delivered: no second wake.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(e.runs().len(), 1);
    assert!(!std::fs::read_to_string(&e.runs).unwrap().contains("SECRET-BODY"));
    stop(tx, h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ordinary_broadcast_does_not_wake_and_a_waking_one_does() {
    let e = env();
    e.session("bob", "opencode", SES_A);
    let (tx, h) = e.start();
    e.broadcast("fyi", false);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(e.runs().is_empty(), "a plain broadcast waits for the next turn");
    e.broadcast("everyone stop", true);
    eventually("woken by the waking broadcast", || e.runs().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(e.runs().len(), 1);
    stop(tx, h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_registered_later_is_picked_up_and_another_watcher_is_never_doubled() {
    let e = env();
    let (tx, h) = e.start();
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Someone else (`corvus watch`) already supervises dave: the leader must not wake him too.
    let dave_lock = WatchLock::acquire(&e.state, &AgentName::parse("dave").unwrap()).unwrap();
    let bob = e.session("bob", "opencode", SES_A);
    let dave = e.session("dave", "opencode", SES_B);
    e.tell(&bob, "hello");
    e.tell(&dave, "hello");
    eventually("bob woken", || e.runs().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(e.runs().len(), 1, "dave is locked by the other watcher");
    assert_eq!(e.runs()[0][0], bob);
    // When the other watcher goes away the leader takes over.
    drop(dave_lock);
    eventually("dave woken after the lock is free", || e.runs().len() == 2).await;
    assert_eq!(e.runs()[1], [dave.as_str(), SES_B, WAKE_NOTE]);
    stop(tx, h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_leader_wakes_and_it_stops_with_the_loop() {
    let e = env();
    let bob = e.session("bob", "opencode", SES_A);
    let timings = LoopTimings {
        first_pass: Duration::from_millis(50),
        interval: Duration::from_millis(100),
        leader_retry: Duration::from_millis(100),
        steps: STEPS,
    };
    let spawn = |rx| {
        tokio::spawn(run_background_with_sources(
            e.t.services(),
            e.state.clone(),
            "test",
            timings,
            rx,
            |_| {},
            None,
            Some(WakeSupervision {
                corvus: e.corvus.clone(),
                timings: fast(),
            }),
        ))
    };
    let (stop1, rx1) = watch::channel(false);
    let (stop2, rx2) = watch::channel(false);
    let (a, b) = (spawn(rx1), spawn(rx2));
    e.tell(&bob, "one");
    eventually("woken", || e.runs().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(e.runs().len(), 1, "two candidates, one wake");

    // Dropping the sender (not sending true) must also end the loop and its watchers.
    drop(stop1);
    stop2.send(true).unwrap();
    for h in [a, b] {
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("background loop stops")
            .unwrap();
    }
    let agent = AgentName::parse(&bob).unwrap();
    assert!(
        WatchLock::acquire(&e.state, &agent).is_ok(),
        "the watcher lock is released"
    );
}
