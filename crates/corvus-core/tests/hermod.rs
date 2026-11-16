//! Plan 0003: Hermod wake-ups and `wait_for_inbox` (D0016). Real database files, real time with short
//! intervals; every wait has an upper bound so a hang fails.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::time::{Duration, Instant};

use corvus_core::api::{CreateThread, PostMessage, RegisterSession, WaitForInbox};
use corvus_core::hermod::{Hermod, HermodConfig, clamp_timeout, wait_for_inbox};
use corvus_core::testing::{TestDb, exec};
use corvus_core::{Services, Settings};
use corvus_domain::MessageType;

fn fast() -> HermodConfig {
    HermodConfig {
        poll: Duration::from_millis(50),
        recheck: Duration::from_secs(60),
        detect: true,
    }
}

fn post(s: &Services, thread: &str, to: Option<&str>, body: &str) -> i64 {
    s.muninn()
        .post_message(&PostMessage {
            thread_id: thread.into(),
            from: "alice".into(),
            body: body.into(),
            kind: MessageType::Note,
            to: to.map(Into::into),
            in_reply_to: None,
            artifacts: vec![],
            idempotency_key: None,
            session_id: None,
            wake: false,
        })
        .unwrap()
        .id
}

struct Rig {
    t: TestDb,
    s: Services,
    other: Services,
    thread: String,
}

fn rig() -> Rig {
    let t = TestDb::new();
    let s = t.services_with(Settings::default());
    let other = t.services_with(Settings::default());
    for a in ["bob", "carol"] {
        s.sessions()
            .register(&RegisterSession {
                agent_name: Some(a.into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap();
    }
    let thread = s
        .muninn()
        .create_thread(&CreateThread {
            creator: "alice".into(),
            title: "t".into(),
            ..Default::default()
        })
        .unwrap()
        .id;
    Rig { t, s, other, thread }
}

fn wait_req(agent: &str, secs: u64) -> WaitForInbox {
    WaitForInbox {
        agent: agent.into(),
        timeout_seconds: Some(secs),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_process_wake_within_detection_interval() {
    let r = rig();
    let h = Hermod::with_config(&r.t.path, fast());
    let (s, h2) = (r.s.clone(), h.clone());
    let waiter = tokio::spawn(async move { wait_for_inbox(&s, &h2, &wait_req("bob", 30)).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let posted_at = Instant::now();
    let id = {
        let other = r.other.clone();
        let th = r.thread.clone();
        tokio::task::spawn_blocking(move || post(&other, &th, Some("bob"), "wake up"))
            .await
            .unwrap()
    };
    let got = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got.iter().map(|m| m.id).collect::<Vec<_>>(), vec![id]);
    assert!(
        posted_at.elapsed() < Duration::from_secs(2),
        "woken by the hint, not the 60 s re-check"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn targeting_other_agents_hint_does_not_wake_but_broadcast_does() {
    let r = rig();
    let h = Hermod::with_config(&r.t.path, fast());
    let sub = h
        .subscribe(&["inbox:bob".into(), "inbox:*".into()])
        .await
        .unwrap();
    post(&r.other, &r.thread, Some("carol"), "not for bob");
    assert!(
        tokio::time::timeout(Duration::from_millis(600), sub.notified())
            .await
            .is_err(),
        "carol's hint must not wake bob"
    );
    post(&r.other, &r.thread, None, "everyone");
    tokio::time::timeout(Duration::from_secs(3), sub.notified())
        .await
        .expect("inbox:* wakes bob");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_returns_empty_at_the_deadline_and_is_clamped() {
    let r = rig();
    let h = Hermod::with_config(&r.t.path, fast());
    let started = Instant::now();
    let got = wait_for_inbox(&r.s, &h, &wait_req("bob", 1)).await.unwrap();
    assert!(got.is_empty());
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_secs(3),
        "{took:?}"
    );
    assert_eq!(clamp_timeout(Some(500)), Duration::from_secs(120));
    assert_eq!(clamp_timeout(Some(0)), Duration::ZERO);
    assert_eq!(clamp_timeout(None), Duration::from_secs(60));
    let zero = Instant::now();
    assert!(
        wait_for_inbox(&r.s, &h, &wait_req("bob", 0))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(zero.elapsed() < Duration::from_millis(500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_committed_before_the_read_is_returned_immediately() {
    let r = rig();
    let h = Hermod::with_config(
        &r.t.path,
        HermodConfig {
            detect: false,
            ..fast()
        },
    );
    let id = post(&r.other, &r.thread, Some("bob"), "already here");
    let started = Instant::now();
    let got = wait_for_inbox(&r.s, &h, &wait_req("bob", 30)).await.unwrap();
    assert_eq!(got[0].id, id);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_hint_with_detection_disabled_returns_via_recheck() {
    let r = rig();
    let h = Hermod::with_config(
        &r.t.path,
        HermodConfig {
            poll: Duration::from_millis(50),
            recheck: Duration::from_millis(300),
            detect: false,
        },
    );
    let (s, h2) = (r.s.clone(), h.clone());
    let waiter = tokio::spawn(async move { wait_for_inbox(&s, &h2, &wait_req("bob", 30)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let id = post(&r.other, &r.thread, Some("bob"), "no hint seen");
    let got = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got[0].id, id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hints_pruned_past_the_cursor_wake_everyone() {
    let r = rig();
    let h = Hermod::with_config(&r.t.path, fast()); // re-check 60 s: only the wake-all can return in time
    let (s, h2) = (r.s.clone(), h.clone());
    let waiter = tokio::spawn(async move { wait_for_inbox(&s, &h2, &wait_req("bob", 30)).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // One transaction: a message for bob (its hint), one for carol, then bob's hint pruned away.
    let th = r.thread.clone();
    let db = r.t.reopen();
    db.write(|tx| {
        let c = tx.sql_mut();
        let ins = |to: &str| {
            c.execute(
                "INSERT INTO messages (thread_id, sender, recipient, type, body, created_at) VALUES (?1, 'alice', ?2, 'note', 'x', 1)",
                [th.as_str(), to],
            )
        };
        ins("bob").map_err(corvus_core::db::sql_err)?;
        ins("carol").map_err(corvus_core::db::sql_err)?;
        c.execute("DELETE FROM hints WHERE topic = 'inbox:bob'", []).map_err(corvus_core::db::sql_err)?;
        Ok(())
    })
    .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_waiter_means_no_polling() {
    let r = rig();
    let h = Hermod::with_config(&r.t.path, fast());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.polls(), 0);
    assert!(!h.watcher_running());
    {
        let _sub = h.subscribe(&["inbox:bob".into()]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(h.polls() > 0);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !h.watcher_running(),
        "the watcher stops when the last subscriber leaves"
    );
    let after = h.polls();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.polls(), after);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coalesced_hints_give_no_duplicates_and_waiting_is_a_heartbeat() {
    let r = rig();
    let sid =
        r.s.sessions()
            .register(&RegisterSession {
                agent_name: Some("bob".into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap()
            .id;
    r.t.clock.advance_secs(500);
    let ids = [
        post(&r.other, &r.thread, Some("bob"), "1"),
        post(&r.other, &r.thread, None, "2"),
    ];
    // Duplicate hints for the same messages.
    exec(
        &r.t.db,
        "INSERT INTO hints (topic) VALUES ('inbox:bob'), ('inbox:bob'), ('inbox:*')",
    )
    .unwrap();
    let h = Hermod::with_config(&r.t.path, fast());
    let req = WaitForInbox {
        session_id: Some(sid.clone()),
        ..wait_req("bob", 5)
    };
    let got = wait_for_inbox(&r.s, &h, &req).await.unwrap();
    assert_eq!(got.iter().map(|m| m.id).collect::<Vec<_>>(), ids.to_vec());
    let session = r.s.sessions().get(&sid).unwrap();
    assert_eq!(session.last_seen_at.0, r.t.now(), "waiting counts as alive");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_with_an_ended_session_is_rejected() {
    let r = rig();
    let sid =
        r.s.sessions()
            .register(&RegisterSession {
                agent_name: Some("bob".into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap()
            .id;
    r.s.sessions().end(&sid).unwrap();
    let h = Hermod::with_config(&r.t.path, fast());
    let req = WaitForInbox {
        session_id: Some(sid),
        ..wait_req("bob", 1)
    };
    assert_eq!(
        wait_for_inbox(&r.s, &h, &req).await.unwrap_err(),
        corvus_domain::CorvusError::SessionNotActive
    );
}
