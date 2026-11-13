//! Plan 0005: housekeeping pass, step isolation, background leader, loop timing.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvus_core::api::{ClaimTask, CreateTask, CreateThread, PostMessage, RegisterSession};
use corvus_core::background::{
    BackgroundLeader, LoopTimings, PassReport, STEPS, Step, StepOutcome, housekeeping_pass,
    housekeeping_pass_with, run_background,
};
use corvus_core::testing::{TestDb, count, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{CorvusError, MessageType};

fn seed(t: &TestDb, s: &Services) {
    let reg = |a: &str| {
        s.sessions()
            .register(&RegisterSession {
                agent_name: Some(a.into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap()
            .id
    };
    let worker = reg("worker");
    let _quiet = reg("quiet");
    s.huginn()
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    s.huginn()
        .claim_task(&ClaimTask {
            session_id: worker.clone(),
            pool: None,
        })
        .unwrap();
    let th = s
        .muninn()
        .create_thread(&CreateThread {
            creator: "op".into(),
            title: "t".into(),
            ..Default::default()
        })
        .unwrap()
        .id;
    for i in 0..1_005 {
        s.muninn()
            .post_message(&PostMessage {
                thread_id: th.clone(),
                from: "op".into(),
                body: format!("m{i}"),
                kind: MessageType::Note,
                to: None,
                in_reply_to: None,
                artifacts: vec![],
                idempotency_key: None,
                session_id: None,
            })
            .unwrap();
    }
    let _ = t;
}

#[test]
fn a_pass_handles_expired_lease_stale_session_and_excess_hints() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        lease_ms: 10_000,
        auto_end_ms: 60_000,
        ..Settings::default()
    });
    seed(&t, &s);
    assert!(count(&t.db, "hints") > 1_000);
    t.clock.advance_secs(61);
    let r = housekeeping_pass(&s);
    assert_eq!(r.count("expire_leases"), Some(1));
    assert_eq!(r.count("end_stale_sessions"), Some(2));
    assert_eq!(
        r.count("prune_hints"),
        Some((count(&t.db, "hints") as usize + r.count("prune_hints").unwrap()) - 1_000)
    );
    assert_eq!(count(&t.db, "hints"), 1_000);
    assert!(r.failed().is_empty());
    assert_eq!(
        query_i64(&t.db, "SELECT count(*) FROM tasks WHERE status = 'open'"),
        Some(1)
    );
}

fn failing(_: &Services) -> Result<usize, CorvusError> {
    Err(CorvusError::Internal("injected failure".into()))
}

fn panicking(_: &Services) -> Result<usize, CorvusError> {
    panic!("injected panic")
}

#[test]
fn a_failing_or_panicking_step_does_not_stop_later_steps() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        lease_ms: 10_000,
        auto_end_ms: 60_000,
        ..Settings::default()
    });
    seed(&t, &s);
    t.clock.advance_secs(61);
    for bad in [failing as fn(&Services) -> Result<usize, CorvusError>, panicking] {
        let steps: Vec<Step> = vec![("expire_leases", bad), STEPS[1], STEPS[2]];
        let r = housekeeping_pass_with(&s, &steps);
        assert_eq!(r.failed(), vec!["expire_leases"]);
        assert!(matches!(&r.steps[0].outcome, StepOutcome::Failed { error } if error.contains("injected")));
        assert!(r.count("prune_hints").is_some(), "later steps still ran");
    }
}

#[test]
fn auto_end_zero_skips_the_session_step() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        auto_end_ms: 0,
        ..Settings::default()
    });
    let r = housekeeping_pass(&s);
    assert!(matches!(r.steps[1].outcome, StepOutcome::Skipped { .. }));
}

#[test]
fn only_one_leader_and_hand_over_on_release() {
    let dir = tempfile::tempdir().unwrap();
    let a = BackgroundLeader::try_acquire(dir.path(), "serve")
        .unwrap()
        .expect("first is leader");
    assert!(
        BackgroundLeader::try_acquire(dir.path(), "desktop")
            .unwrap()
            .is_none(),
        "second waits"
    );
    let info = BackgroundLeader::read_info(dir.path()).unwrap();
    assert_eq!((info.pid, info.kind.as_str()), (std::process::id(), "serve"));
    drop(a);
    assert!(BackgroundLeader::read_info(dir.path()).is_none());
    let b = BackgroundLeader::try_acquire(dir.path(), "desktop")
        .unwrap()
        .expect("takes over");
    assert_eq!(BackgroundLeader::read_info(dir.path()).unwrap().kind, "desktop");
    drop(b);
}

#[tokio::test(start_paused = true)]
async fn passes_fire_at_5_65_125_seconds() {
    let t = TestDb::new();
    let s = t.services();
    let dir = tempfile::tempdir().unwrap();
    let times = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let start = tokio::time::Instant::now();
    let t2 = times.clone();
    let h = tokio::spawn(run_background(
        s,
        dir.path().to_path_buf(),
        "test",
        LoopTimings::default(),
        rx,
        move |_r: &PassReport| {
            t2.lock().unwrap().push(start.elapsed().as_secs());
        },
    ));
    // Advance in small steps so blocking work (spawn_blocking) completes between ticks.
    for _ in 0..130 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tx.send(true).unwrap();
    h.await.unwrap();
    let got = times.lock().unwrap().clone();
    assert_eq!(got, vec![5, 65, 125], "{got:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_leader_takes_over_when_the_leader_stops() {
    let t = TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    let fast = LoopTimings {
        first_pass: Duration::from_millis(10),
        interval: Duration::from_millis(50),
        leader_retry: Duration::from_millis(100),
        ..LoopTimings::default()
    };
    let count_a = Arc::new(Mutex::new(0));
    let count_b = Arc::new(Mutex::new(0));
    let (tx_a, rx_a) = tokio::sync::watch::channel(false);
    let (tx_b, rx_b) = tokio::sync::watch::channel(false);
    let ca = count_a.clone();
    let a = tokio::spawn(run_background(
        t.services(),
        dir.path().to_path_buf(),
        "a",
        fast,
        rx_a,
        move |_| *ca.lock().unwrap() += 1,
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cb = count_b.clone();
    let b = tokio::spawn(run_background(
        t.services(),
        dir.path().to_path_buf(),
        "b",
        fast,
        rx_b,
        move |_| *cb.lock().unwrap() += 1,
    ));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(*count_a.lock().unwrap() > 0);
    assert_eq!(*count_b.lock().unwrap(), 0, "only the leader runs passes");
    assert_eq!(BackgroundLeader::read_info(dir.path()).unwrap().kind, "a");
    tx_a.send(true).unwrap();
    a.await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        *count_b.lock().unwrap() > 0,
        "b took over within the retry interval"
    );
    assert_eq!(BackgroundLeader::read_info(dir.path()).unwrap().kind, "b");
    tx_b.send(true).unwrap();
    b.await.unwrap();
}

#[test]
fn without_a_background_process_claim_still_reaps() {
    let t = TestDb::new();
    let s = t.services_with(Settings {
        lease_ms: 10_000,
        ..Settings::default()
    });
    let reg = |a: &str| {
        s.sessions()
            .register(&RegisterSession {
                agent_name: Some(a.into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap()
            .id
    };
    let (a, b) = (reg("a"), reg("b"));
    s.huginn()
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    s.huginn()
        .claim_task(&ClaimTask {
            session_id: a,
            pool: None,
        })
        .unwrap();
    t.clock.advance_secs(10);
    assert!(
        s.huginn()
            .claim_task(&ClaimTask {
                session_id: b,
                pool: None
            })
            .unwrap()
            .task
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loop_keeps_ticking_through_failing_and_panicking_steps() {
    fn boom(_: &Services) -> Result<usize, CorvusError> {
        Err(CorvusError::Busy)
    }
    fn kaboom(_: &Services) -> Result<usize, CorvusError> {
        panic!("step blew up")
    }
    static BAD: &[Step] = &[("expire_leases", boom), ("end_stale_sessions", kaboom), STEPS[2]];
    let t = TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    let timings = LoopTimings {
        first_pass: Duration::from_millis(10),
        interval: Duration::from_millis(30),
        leader_retry: Duration::from_millis(50),
        steps: BAD,
    };
    let reports = Arc::new(Mutex::new(Vec::<PassReport>::new()));
    let r2 = reports.clone();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_background(
        t.services(),
        dir.path().to_path_buf(),
        "t",
        timings,
        rx,
        move |r| {
            r2.lock().unwrap().push(r.clone());
        },
    ));
    tokio::time::sleep(Duration::from_millis(400)).await;
    tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), h)
        .await
        .expect("stops promptly")
        .unwrap();
    let reports = reports.lock().unwrap();
    assert!(reports.len() >= 3, "kept ticking: {} passes", reports.len());
    for r in reports.iter() {
        assert_eq!(r.failed(), vec!["expire_leases", "end_stale_sessions"]);
        assert!(r.count("prune_hints").is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loop_waits_out_a_held_write_lock_and_recovers() {
    let t = TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    let timings = LoopTimings {
        first_pass: Duration::from_millis(10),
        interval: Duration::from_millis(50),
        leader_retry: Duration::from_millis(50),
        ..LoopTimings::default()
    };
    let passes = Arc::new(Mutex::new(Vec::<std::time::Instant>::new()));
    let p2 = passes.clone();
    // Another connection holds the database's write lock for 1.5 s, as a stray tool might.
    let blocker = t.reopen();
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let hold = std::thread::spawn(move || {
        blocker
            .write(|_| {
                held_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(1500));
                Ok(())
            })
            .unwrap();
    });
    held_rx.recv().unwrap();
    let released_at = std::time::Instant::now() + Duration::from_millis(1500);
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_background(
        t.services(),
        dir.path().to_path_buf(),
        "t",
        timings,
        rx,
        move |_| {
            p2.lock().unwrap().push(std::time::Instant::now());
        },
    ));
    tokio::time::sleep(Duration::from_millis(2500)).await;
    tx.send(true).unwrap();
    h.await.unwrap();
    hold.join().unwrap();
    let passes = passes.lock().unwrap();
    assert!(
        passes.iter().filter(|p| **p > released_at).count() >= 3,
        "recovered and kept ticking after the lock"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_shutdown_sender_stops_the_loop() {
    let t = TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_background(
        t.services(),
        dir.path().to_path_buf(),
        "t",
        LoopTimings::default(),
        rx,
        |_| {},
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(tx);
    tokio::time::timeout(Duration::from_secs(2), h)
        .await
        .expect("stops when its owner is gone")
        .unwrap();
}
