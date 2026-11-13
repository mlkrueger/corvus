//! Plan 0002: sessions, presence, auto-end (functional; races live in races.rs).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::api::{ClaimTask, CreateTask, ListSessions, RegisterSession};
use corvus_core::testing::{TestDb, count, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{CorvusError, Presence, SessionEndReason, TaskStatus};

fn reg(s: &Services, name: Option<&str>) -> corvus_core::api::SessionView {
    s.sessions()
        .register(&RegisterSession {
            agent_name: name.map(str::to_owned),
            client_name: "test".into(),
            ..Default::default()
        })
        .unwrap()
}

fn events(t: &TestDb, kind: &str) -> i64 {
    query_i64(
        &t.db,
        &format!("SELECT count(*) FROM events WHERE kind = '{kind}'"),
    )
    .unwrap()
}

#[test]
fn register_named_creates_agent_session_and_one_event() {
    let t = TestDb::new();
    let s = t.services();
    let v = s
        .sessions()
        .register(&RegisterSession {
            agent_name: Some("alice".into()),
            client_name: "claude-code".into(),
            metadata: Some(serde_json::json!({"cwd": "/repo"})),
            idempotency_key: None,
        })
        .unwrap();
    assert_eq!(v.agent_name, "alice");
    assert!(!v.name_generated);
    assert_eq!(v.presence, Presence::Active);
    assert_eq!(v.metadata["cwd"], "/repo");
    assert_eq!(v.started_at.0, t.now());
    assert_eq!(count(&t.db, "agents"), 1);
    assert_eq!(events(&t, "session.registered"), 1);
    // A second session for the same agent is a different session.
    let v2 = reg(&s, Some("alice"));
    assert_ne!(v.id, v2.id);
    assert_eq!(count(&t.db, "agents"), 1);
}

#[test]
fn register_without_name_generates_adjective_bird() {
    let t = TestDb::new();
    let v = reg(&t.services(), None);
    assert!(v.name_generated);
    let (adj, bird) = v.agent_name.split_once('-').unwrap();
    assert!(corvus_domain::names::ADJECTIVES.contains(&adj), "{adj}");
    assert!(corvus_domain::names::BIRDS.contains(&bird), "{bird}");
}

#[test]
fn register_validation() {
    let t = TestDb::new();
    let s = t.services();
    let bad = [
        RegisterSession {
            agent_name: Some("-x".into()),
            client_name: "c".into(),
            ..Default::default()
        },
        RegisterSession {
            agent_name: Some("a".into()),
            client_name: "  ".into(),
            ..Default::default()
        },
        RegisterSession {
            agent_name: Some("a".into()),
            client_name: "c".into(),
            metadata: Some(serde_json::json!([1])),
            ..Default::default()
        },
        RegisterSession {
            agent_name: Some("a".into()),
            client_name: "c".into(),
            metadata: Some(serde_json::json!({"x": "y".repeat(20_000)})),
            ..Default::default()
        },
    ];
    for r in bad {
        assert!(
            matches!(s.sessions().register(&r), Err(CorvusError::Validation(_))),
            "{r:?}"
        );
    }
    assert_eq!(count(&t.db, "sessions"), 0);
}

#[test]
fn register_idempotent_replay_returns_original() {
    let t = TestDb::new();
    let s = t.services();
    let r = |client: &str| RegisterSession {
        agent_name: Some("alice".into()),
        client_name: client.into(),
        idempotency_key: Some("k1".into()),
        ..Default::default()
    };
    let a = s.sessions().register(&r("one")).unwrap();
    let b = s.sessions().register(&r("two")).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(
        b.client_name, "one",
        "the key is the identity; the replay returns the original"
    );
    assert_eq!(count(&t.db, "sessions"), 1);
    assert_eq!(events(&t, "session.registered"), 1);
    // Same key, different agent: a different session.
    let c = s
        .sessions()
        .register(&RegisterSession {
            agent_name: Some("bob".into()),
            ..r("x")
        })
        .unwrap();
    assert_ne!(a.id, c.id);
}

#[test]
fn generated_name_registration_is_idempotent_by_key() {
    let t = TestDb::new();
    let s = t.services();
    let r = RegisterSession {
        client_name: "c".into(),
        idempotency_key: Some("gen".into()),
        ..Default::default()
    };
    let a = s.sessions().register(&r).unwrap();
    let b = s.sessions().register(&r).unwrap();
    assert_eq!((a.id, a.agent_name), (b.id, b.agent_name));
    assert_eq!(count(&t.db, "agents"), 1);
}

#[test]
fn heartbeat_bumps_last_seen_without_an_event() {
    let t = TestDb::new();
    let s = t.services();
    let v = reg(&s, Some("alice"));
    let before = count(&t.db, "events");
    t.clock.advance_secs(30);
    let h = s.sessions().heartbeat(&v.id).unwrap();
    assert_eq!(h.last_seen_at.0, t.now());
    assert_eq!(count(&t.db, "events"), before);
}

#[test]
fn unknown_or_ended_session_is_not_active() {
    let t = TestDb::new();
    let s = t.services();
    let unknown = "01890000-0000-7000-8000-000000000000";
    assert_eq!(
        s.sessions().heartbeat(unknown).unwrap_err(),
        CorvusError::SessionNotActive
    );
    assert_eq!(
        s.sessions().end(unknown).unwrap_err(),
        CorvusError::SessionNotActive
    );
    let v = reg(&s, Some("alice"));
    s.sessions().end(&v.id).unwrap();
    assert_eq!(
        s.sessions().heartbeat(&v.id).unwrap_err(),
        CorvusError::SessionNotActive
    );
    let e = s
        .huginn()
        .claim_task(&ClaimTask {
            session_id: v.id.clone(),
            pool: None,
        })
        .unwrap_err();
    assert_eq!(e, CorvusError::SessionNotActive);
    assert!(e.to_string().contains("register_session again"));
    assert!(matches!(
        s.sessions().heartbeat("not-a-uuid"),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn end_session_releases_its_task_and_is_idempotent() {
    let t = TestDb::new();
    let s = t.services();
    let v = reg(&s, Some("alice"));
    s.huginn()
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    let c = s
        .huginn()
        .claim_task(&ClaimTask {
            session_id: v.id.clone(),
            pool: None,
        })
        .unwrap();
    let task_id = c.task.unwrap().id;
    let before = count(&t.db, "events");
    let ended = s.sessions().end(&v.id).unwrap();
    assert_eq!(ended.presence, Presence::Ended);
    assert_eq!(ended.end_reason, Some(SessionEndReason::Ended));
    assert_eq!(ended.current_task, None);
    let task = s.huginn().get_task(&task_id).unwrap();
    assert_eq!(task.status, TaskStatus::Open);
    assert!(task.holder.is_none());
    assert_eq!(
        query_i64(
            &t.db,
            "SELECT count(*) FROM claims WHERE end_reason = 'released' AND released_at IS NOT NULL"
        ),
        Some(1)
    );
    assert_eq!(count(&t.db, "events"), before + 1, "one event for the whole end");
    // Replay: no-op, no event.
    let again = s.sessions().end(&v.id).unwrap();
    assert_eq!(again.ended_at, ended.ended_at);
    assert_eq!(count(&t.db, "events"), before + 1);
}

#[test]
fn presence_follows_the_clock_and_claims() {
    let t = TestDb::new();
    let s = t.services();
    let v = reg(&s, Some("alice"));
    let p = |s: &Services| s.sessions().get(&v.id).unwrap().presence;
    assert_eq!(p(&s), Presence::Active);
    t.clock.advance_secs(120);
    assert_eq!(p(&s), Presence::Active, "within the active window is inclusive");
    t.clock.advance_ms(1);
    assert_eq!(p(&s), Presence::Idle);
    t.clock.advance_ms(900_000 - 120_001);
    assert_eq!(
        p(&s),
        Presence::Idle,
        "exactly the idle window is not yet offline"
    );
    t.clock.advance_ms(1);
    assert_eq!(p(&s), Presence::Offline);

    s.sessions().heartbeat(&v.id).unwrap();
    s.huginn()
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    s.huginn()
        .claim_task(&ClaimTask {
            session_id: v.id.clone(),
            pool: None,
        })
        .unwrap();
    t.clock.advance_secs(300);
    assert_eq!(p(&s), Presence::Working);
    s.sessions().end(&v.id).unwrap();
    assert_eq!(p(&s), Presence::Ended);
}

#[test]
fn list_sessions_filters() {
    let t = TestDb::new();
    let s = t.services();
    let a = reg(&s, Some("alice"));
    reg(&s, Some("bob"));
    s.sessions().end(&a.id).unwrap();
    assert_eq!(s.sessions().list(&ListSessions::default()).unwrap().len(), 1);
    assert_eq!(
        s.sessions()
            .list(&ListSessions {
                include_ended: true,
                agent: None
            })
            .unwrap()
            .len(),
        2
    );
    let only = s
        .sessions()
        .list(&ListSessions {
            include_ended: true,
            agent: Some("alice".into()),
        })
        .unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].agent_name, "alice");
}

#[test]
fn end_stale_ends_silent_sessions_and_releases_claims() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        auto_end_ms: 3_600_000,
        ..Settings::default()
    });
    let quiet = reg(&s, Some("quiet"));
    let busy = reg(&s, Some("busy"));
    s.huginn()
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    s.huginn()
        .claim_task(&ClaimTask {
            session_id: quiet.id.clone(),
            pool: None,
        })
        .unwrap();

    t.clock.advance_secs(3_600);
    s.sessions().heartbeat(&busy.id).unwrap();
    assert!(
        s.sessions().end_stale().unwrap().ended.is_empty(),
        "exactly the threshold is not stale"
    );
    t.clock.advance_ms(1);
    let r = s.sessions().end_stale().unwrap();
    assert_eq!(r.ended, vec![quiet.id.clone()]);
    let q = s.sessions().get(&quiet.id).unwrap();
    assert_eq!(q.end_reason, Some(SessionEndReason::AutoEnded));
    assert_eq!(
        s.huginn().list_tasks(&Default::default()).unwrap()[0].status,
        TaskStatus::Open
    );
    assert_eq!(events(&t, "session.auto_ended"), 1);
    assert!(s.sessions().end_stale().unwrap().ended.is_empty());
}

#[test]
fn end_stale_disabled_by_zero() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        auto_end_ms: 0,
        ..Settings::default()
    });
    reg(&s, Some("quiet"));
    t.clock.advance_secs(1_000_000);
    assert!(s.sessions().end_stale().unwrap().ended.is_empty());
}
