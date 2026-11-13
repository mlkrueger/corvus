//! Plan 0002 verification: concurrency races between threads.
//!
//! Technique ([system/09](../../../spec/system/09-testing.md#concurrency-test-technique)): every racing thread
//! has its OWN `Database` handle (its own connections) on one real database file; all handles share one
//! `ManualClock`; threads are released together by a `Barrier`; scenarios repeat many times; assertions are on
//! the database state (claims, tasks, events, messages), not only on return values; no operation may return
//! `Busy`; every race has a timeout so a hang fails the test.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::any::Any;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use corvus_core::ReadOnlyDatabase;
use corvus_core::api::{CancelTask, ClaimTask, CompleteTask, CreateTask, RegisterSession};
use corvus_core::db::{Reader, SqlResultExt};
use corvus_core::testing::exec;
use corvus_core::{Clock, Database, ManualClock, Services, Settings};
use corvus_domain::CorvusError;
use rusqlite::params_from_iter;

/// How long one race may take before it counts as hung.
const RACE_TIMEOUT: Duration = Duration::from_secs(30);

// ---------- rig ----------

/// A migrated database file in a temp directory, plus a shared manual clock.
struct Rig {
    _dir: tempfile::TempDir,
    path: PathBuf,
    clock: Arc<ManualClock>,
    settings: Settings,
}

impl Rig {
    fn new() -> Self {
        Self::with(Settings::default())
    }

    fn with(settings: Settings) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("corvus.db");
        let clock = Arc::new(ManualClock::default());
        drop(Database::open(&path, clock.as_ref()).unwrap()); // migrate once, up front
        Self {
            _dir: dir,
            path,
            clock,
            settings,
        }
    }

    /// A service layer on its own, independent database handle.
    fn handle(&self) -> Services {
        let (db, _) = Database::open(&self.path, self.clock.as_ref()).unwrap();
        Services::new(Arc::new(db), self.clock.clone(), self.settings)
    }

    fn handles(&self, n: usize) -> Vec<Services> {
        (0..n).map(|_| self.handle()).collect()
    }

    /// A read-only handle (opened by the one opener) for state assertions.
    fn raw(&self) -> ReadOnlyDatabase {
        ReadOnlyDatabase::open(&self.path, 5000).unwrap()
    }

    /// One integer, with string parameters.
    fn n(&self, sql: &str, args: &[&str]) -> i64 {
        self.raw()
            .read(|tx| {
                tx.sql()
                    .query_row(sql, params_from_iter(args.iter()), |r| r.get(0))
                    .db()
            })
            .unwrap()
    }

    fn text(&self, sql: &str, args: &[&str]) -> Option<String> {
        self.raw()
            .read(|tx| {
                tx.sql()
                    .query_row(sql, params_from_iter(args.iter()), |r| r.get(0))
                    .db()
            })
            .unwrap()
    }
}

// ---------- racing ----------

fn panic_text(p: &(dyn Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".into())
}

/// Runs `f(i, handle_i)` on one thread per handle, all released together by a barrier. Fails on a hang or on
/// a panic inside any worker. Results are returned in handle order.
fn race<T: Send + 'static>(
    handles: &[Services],
    f: impl Fn(usize, &Services) -> T + Send + Sync + 'static,
) -> Vec<T> {
    race_for(handles, RACE_TIMEOUT, f)
}

fn race_for<T: Send + 'static>(
    handles: &[Services],
    timeout: Duration,
    f: impl Fn(usize, &Services) -> T + Send + Sync + 'static,
) -> Vec<T> {
    let barrier = Arc::new(Barrier::new(handles.len()));
    let f = Arc::new(f);
    let (tx, rx) = mpsc::channel();
    for (i, h) in handles.iter().cloned().enumerate() {
        let (barrier, f, tx) = (barrier.clone(), f.clone(), tx.clone());
        std::thread::spawn(move || {
            barrier.wait();
            let r = catch_unwind(AssertUnwindSafe(|| f(i, &h)));
            let _ = tx.send((i, r));
        });
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let mut out: Vec<Option<T>> = (0..handles.len()).map(|_| None).collect();
    for _ in 0..handles.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((i, Ok(v))) => out[i] = Some(v),
            Ok((i, Err(p))) => panic!("race worker {i} panicked: {}", panic_text(p.as_ref())),
            Err(RecvTimeoutError::Timeout) => panic!("race hung: no result within {timeout:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("race workers vanished"),
        }
    }
    out.into_iter().map(Option::unwrap).collect()
}

/// A worker error that must never happen in these tests.
fn never_busy<T>(r: &Result<T, CorvusError>, what: &str) {
    if let Err(e @ (CorvusError::Busy | CorvusError::Internal(_) | CorvusError::Validation(_))) = r {
        panic!("{what} returned unexpected {e:?}");
    }
}

// ---------- fixtures ----------

fn register(svc: &Services, agent: &str) -> String {
    svc.sessions()
        .register(&RegisterSession {
            agent_name: Some(agent.to_owned()),
            client_name: "races".into(),
            ..RegisterSession::default()
        })
        .unwrap()
        .id
}

fn sessions(svc: &Services, prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| register(svc, &format!("{prefix}-{i}"))).collect()
}

fn new_task(svc: &Services, title: &str) -> String {
    svc.huginn()
        .create_task(&CreateTask {
            creator: "operator".into(),
            title: title.into(),
            ..CreateTask::default()
        })
        .unwrap()
        .id
}

fn claim(svc: &Services, session: &str) -> Result<corvus_core::api::ClaimResult, CorvusError> {
    svc.huginn().claim_task(&ClaimTask {
        session_id: session.to_owned(),
        pool: None,
    })
}

fn complete(svc: &Services, task: &str, session: &str) {
    svc.huginn()
        .complete_task(
            task,
            &CompleteTask {
                session_id: session.to_owned(),
                result_summary: None,
            },
        )
        .unwrap();
}

fn cancel(svc: &Services, task: &str) -> Result<corvus_core::api::CancelResult, CorvusError> {
    svc.huginn().cancel_task(
        task,
        &CancelTask {
            by: "operator".into(),
            reason: Some("race".into()),
            ..CancelTask::default()
        },
    )
}

/// Structural invariants of the work queue, evaluated in ONE statement (one consistent snapshot). Returns
/// the names of violated invariants with their violation counts.
fn violations(db: &ReadOnlyDatabase) -> Vec<(&'static str, i64)> {
    const CHECKS: [&str; 6] = [
        "two_active_claims_on_one_task",
        "two_active_claims_in_one_session",
        "claimed_task_without_exactly_one_active_claim",
        "active_claim_on_non_claimed_task",
        "active_claim_held_by_ended_session",
        "ended_before_last_seen",
    ];
    let sql = "SELECT
      (SELECT count(*) FROM (SELECT 1 FROM claims WHERE released_at IS NULL GROUP BY task_id HAVING count(*) > 1)),
      (SELECT count(*) FROM (SELECT 1 FROM claims WHERE released_at IS NULL GROUP BY session_id HAVING count(*) > 1)),
      (SELECT count(*) FROM tasks t WHERE t.status = 'claimed'
         AND (SELECT count(*) FROM claims c WHERE c.task_id = t.id AND c.released_at IS NULL) <> 1),
      (SELECT count(*) FROM claims c JOIN tasks t ON t.id = c.task_id
         WHERE c.released_at IS NULL AND t.status <> 'claimed'),
      (SELECT count(*) FROM claims c JOIN sessions s ON s.id = c.session_id
         WHERE c.released_at IS NULL AND s.ended_at IS NOT NULL),
      (SELECT count(*) FROM sessions WHERE ended_at IS NOT NULL AND last_seen_at > ended_at)";
    let counts: Vec<i64> = db
        .read(|tx| {
            tx.sql()
                .query_row(sql, [], |r| (0..CHECKS.len()).map(|i| r.get(i)).collect())
                .db()
        })
        .unwrap();
    CHECKS
        .iter()
        .zip(counts)
        .filter(|(_, c)| *c != 0)
        .map(|(n, c)| (*n, c))
        .collect()
}

fn assert_invariants(rig: &Rig, when: &str) {
    let v = violations(&rig.raw());
    assert!(v.is_empty(), "invariants violated {when}: {v:?}");
}

// ---------- 1. claim race (threads) ----------

#[test]
fn claim_race_threads_exactly_one_winner() {
    const N: usize = 24;
    let rig = Rig::new();
    let hs = rig.handles(N);
    let sess = Arc::new(sessions(&hs[0], "racer", N));
    for iter in 0..40 {
        let task = new_task(&hs[0], &format!("race {iter}"));
        let s = sess.clone();
        let results = race(&hs, move |i, svc| claim(svc, &s[i]));

        let mut winners = Vec::new();
        for (i, r) in results.iter().enumerate() {
            never_busy(r, "claim_task");
            let r = r
                .as_ref()
                .unwrap_or_else(|e| panic!("iter {iter}: claim {i} failed: {e:?}"));
            match (&r.task, &r.claim) {
                (Some(t), Some(c)) => {
                    assert_eq!(t.id, task);
                    assert_eq!(c.task_id, task);
                    assert_eq!(c.session_id, sess[i]);
                    winners.push(i);
                }
                (None, None) => {}
                other => panic!("task and claim must be both set or both unset: {other:?}"),
            }
        }
        assert_eq!(
            winners.len(),
            1,
            "iter {iter}: exactly one winner, got {winners:?}"
        );
        let w = winners[0];

        // Database state.
        assert_eq!(
            rig.n("SELECT count(*) FROM claims WHERE task_id = ?1", &[&task]),
            1
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM claims WHERE task_id = ?1 AND released_at IS NULL AND session_id = ?2",
                &[&task, &sess[w]]
            ),
            1
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL", &[]),
            1
        );
        assert_eq!(
            rig.text("SELECT status FROM tasks WHERE id = ?1", &[&task])
                .unwrap(),
            "claimed"
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM events WHERE kind = 'task.claimed' AND task_id = ?1",
                &[&task]
            ),
            1
        );
        assert_invariants(&rig, "after claim race");
        complete(&hs[0], &task, &sess[w]);
    }
}

// ---------- 3. claim fan-out ----------

#[test]
fn claim_fan_out_n_claimers_n_tasks_n_distinct_tasks() {
    const N: usize = 16;
    let rig = Rig::new();
    let hs = rig.handles(N);
    let sess = Arc::new(sessions(&hs[0], "fan", N));
    for iter in 0..25 {
        let tasks: HashSet<String> = (0..N)
            .map(|k| new_task(&hs[0], &format!("fan {iter}.{k}")))
            .collect();
        let s = sess.clone();
        let results = race(&hs, move |i, svc| claim(svc, &s[i]));

        let mut got = Vec::new();
        for (i, r) in results.iter().enumerate() {
            never_busy(r, "claim_task");
            let t = r.as_ref().unwrap().task.as_ref().unwrap_or_else(|| {
                panic!("iter {iter}: claimer {i} got nothing although {N} tasks were open")
            });
            got.push(t.id.clone());
        }
        let distinct: HashSet<String> = got.iter().cloned().collect();
        assert_eq!(
            distinct.len(),
            N,
            "iter {iter}: every claimer got a different task"
        );
        assert_eq!(distinct, tasks);

        assert_eq!(
            rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL", &[]),
            N as i64
        );
        assert_eq!(
            rig.n(
                "SELECT count(DISTINCT task_id) FROM claims WHERE released_at IS NULL",
                &[]
            ),
            N as i64
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM tasks WHERE status = 'claimed'", &[]),
            N as i64
        );
        for t in &got {
            assert_eq!(
                rig.n(
                    "SELECT count(*) FROM events WHERE kind = 'task.claimed' AND task_id = ?1",
                    &[t]
                ),
                1
            );
        }
        assert_invariants(&rig, "after fan-out");
        for (i, t) in got.iter().enumerate() {
            complete(&hs[0], t, &sess[i]);
        }
    }
}

// ---------- 4. queueing is bounded ----------

#[test]
fn queueing_bounded_write_hold_and_wait_stay_under_ceiling() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 40;
    const MAX_HOLD_US: u64 = 250_000;
    const MAX_WAIT_US: u64 = 5_000_000;
    let rig = Rig::new();
    let hs = rig.handles(THREADS);
    let sess = Arc::new(sessions(&hs[0], "stress", THREADS));
    for h in &hs {
        h.db().reset_write_stats();
    }
    let ops = Arc::new(AtomicUsize::new(0));
    let o = ops.clone();
    let s = sess.clone();
    let results = race_for(&hs, Duration::from_secs(120), move |i, svc| {
        let h = svc.huginn();
        let count = |n: usize| {
            o.fetch_add(n, Ordering::Relaxed);
        };
        for k in 0..ROUNDS {
            let created = h
                .create_task(&CreateTask {
                    creator: format!("maker-{i}"),
                    title: format!("t{i}.{k}"),
                    ..CreateTask::default()
                })
                .map(|_| ());
            never_busy(&created, "create_task");
            created.unwrap();
            let r = claim(svc, &s[i]);
            never_busy(&r, "claim_task");
            count(2);
            if let Some(t) = r.unwrap().task {
                match k % 3 {
                    0 => {
                        let renewed = h.renew_lease(&t.id, &s[i]);
                        never_busy(&renewed, "renew_lease");
                        renewed.unwrap();
                        complete(svc, &t.id, &s[i]);
                        count(2);
                    }
                    1 => {
                        let released = h.release_task(&t.id, &s[i]);
                        never_busy(&released, "release_task");
                        released.unwrap();
                        count(1);
                    }
                    _ => {
                        complete(svc, &t.id, &s[i]);
                        count(1);
                    }
                }
            }
        }
    });
    assert_eq!(results.len(), THREADS);

    let (mut hold, mut wait, mut txs) = (0, 0, 0);
    for h in &hs {
        let st = h.db().write_stats();
        hold = hold.max(st.max_hold_us);
        wait = wait.max(st.max_wait_us);
        txs += st.transactions;
    }
    eprintln!(
        "queueing_bounded: {txs} write transactions on {THREADS} handles; max_hold_us={hold} max_wait_us={wait}"
    );
    assert!(
        txs >= (THREADS * ROUNDS * 2) as u64,
        "stress run did real work: {txs}"
    );
    assert!(
        hold < MAX_HOLD_US,
        "longest write transaction {hold} us >= {MAX_HOLD_US} us"
    );
    assert!(
        wait < MAX_WAIT_US,
        "longest wait for the write lock {wait} us >= {MAX_WAIT_US} us"
    );
    assert_eq!(
        rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL", &[]),
        0
    );
    assert_invariants(&rig, "after stress");
}

// ---------- 5. cancel versus claim (threads) ----------

#[test]
fn cancel_vs_claim_threads_only_serial_outcomes() {
    const CLAIMERS: usize = 8;
    const CANCELLERS: usize = 8;
    let rig = Rig::new();
    let hs = rig.handles(CLAIMERS + CANCELLERS);
    let sess = Arc::new(sessions(&hs[0], "cvc", CLAIMERS));
    let (mut claimed_then_cancelled, mut cancelled_first) = (0, 0);
    for iter in 0..120 {
        let task = new_task(&hs[0], &format!("cvc {iter}"));
        let (s, t) = (sess.clone(), task.clone());
        let results = race(&hs, move |i, svc| {
            if i < CLAIMERS {
                (Some(claim(svc, &s[i])), None)
            } else {
                (None, Some(cancel(svc, &t)))
            }
        });

        let mut winner = None;
        let mut cancel_effective = 0;
        for (i, (c, x)) in results.iter().enumerate() {
            if let Some(c) = c {
                never_busy(c, "claim_task");
                if c.as_ref().unwrap().task.is_some() {
                    assert!(winner.replace(i).is_none(), "two winners");
                }
            }
            if let Some(x) = x {
                never_busy(x, "cancel_task");
                let x = x.as_ref().unwrap();
                assert_eq!(x.task.id, task);
                if !x.cancelled.is_empty() {
                    assert_eq!(x.cancelled, vec![task.clone()]);
                    cancel_effective += 1;
                }
            }
        }
        assert_eq!(
            cancel_effective, 1,
            "iter {iter}: exactly one cancel takes effect"
        );

        // Database state: cancelled; never a live claim.
        assert_eq!(
            rig.text("SELECT status FROM tasks WHERE id = ?1", &[&task])
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM claims WHERE task_id = ?1 AND released_at IS NULL",
                &[&task]
            ),
            0,
            "iter {iter}: live claim on a cancelled task"
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM events WHERE kind = 'task.cancelled' AND task_id = ?1",
                &[&task]
            ),
            1
        );
        let status_msgs = "SELECT count(*) FROM messages m JOIN threads th ON th.id = m.thread_id
                           WHERE th.task_id = ?1 AND m.type = 'status'";
        match winner {
            Some(w) => {
                // (a) claimed, then cancelled.
                claimed_then_cancelled += 1;
                assert_eq!(
                    rig.n("SELECT count(*) FROM claims WHERE task_id = ?1", &[&task]),
                    1
                );
                assert_eq!(
                    rig.n(
                        "SELECT count(*) FROM claims WHERE task_id = ?1 AND session_id = ?2
                         AND end_reason = 'cancelled' AND released_at IS NOT NULL",
                        &[&task, &sess[w]]
                    ),
                    1,
                    "iter {iter}: claim must end 'cancelled'"
                );
                let who = format!("cvc-{w}");
                assert_eq!(
                    rig.n(&format!("{status_msgs} AND m.recipient = ?2"), &[&task, &who]),
                    1,
                    "iter {iter}: claimant gets one status message"
                );
                assert_eq!(rig.n(status_msgs, &[&task]), 1);
            }
            None => {
                // (b) cancelled and never claimed.
                cancelled_first += 1;
                assert_eq!(
                    rig.n("SELECT count(*) FROM claims WHERE task_id = ?1", &[&task]),
                    0
                );
                assert_eq!(rig.n(status_msgs, &[&task]), 0);
                assert_eq!(
                    rig.n(
                        "SELECT count(*) FROM events WHERE kind = 'task.claimed' AND task_id = ?1",
                        &[&task]
                    ),
                    0
                );
            }
        }
        assert_invariants(&rig, "after cancel-vs-claim");
    }
    eprintln!(
        "cancel_vs_claim: claimed-then-cancelled={claimed_then_cancelled} cancelled-first={cancelled_first}"
    );
}

// ---------- 6. mixed stress ----------

struct Shared {
    tasks: Mutex<Vec<String>>,
    clock: Arc<ManualClock>,
    path: PathBuf,
    lease_secs: i64,
}

/// Tiny deterministic generator (no extra dependencies).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Allowed outcomes of a racing operation: success, or a normal refusal.
fn tolerate<T>(r: Result<T, CorvusError>, what: &str) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(CorvusError::Conflict(_) | CorvusError::SessionNotActive | CorvusError::NotFound(_)) => None,
        Err(e) => panic!("{what} returned unexpected {e:?}"),
    }
}

fn mixed_worker(i: usize, svc: &Services, shared: &Shared, ops: usize) -> usize {
    let h = svc.huginn();
    let conn = ReadOnlyDatabase::open(&shared.path, 5000).unwrap();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((i as u64 + 1) * 0x1000_0001));
    let mut session = register(svc, &format!("mixed-{i}-0"));
    let mut generation = 0;
    let mut held: Option<String> = None;
    let pick = |rng: &mut Rng, held: &Option<String>| -> Option<String> {
        held.clone().or_else(|| {
            let t = shared.tasks.lock().unwrap();
            (!t.is_empty()).then(|| t[rng.below(t.len())].clone())
        })
    };
    let mut new_session = |session: &mut String, held: &mut Option<String>| {
        generation += 1;
        *session = register(svc, &format!("mixed-{i}-{generation}"));
        *held = None;
    };
    for k in 0..ops {
        let roll = rng.below(100);
        match roll {
            0..30 => match tolerate(claim(svc, &session), "claim_task") {
                Some(r) => held = r.task.map(|t| t.id).or(held),
                None => new_session(&mut session, &mut held),
            },
            30..40 => {
                if let Some(t) = pick(&mut rng, &held) {
                    tolerate(h.renew_lease(&t, &session), "renew_lease");
                }
            }
            40..50 => {
                if let Some(t) = pick(&mut rng, &held) {
                    tolerate(h.release_task(&t, &session), "release_task");
                }
            }
            50..60 => {
                if let Some(t) = pick(&mut rng, &held) {
                    tolerate(
                        h.complete_task(
                            &t,
                            &CompleteTask {
                                session_id: session.clone(),
                                result_summary: None,
                            },
                        ),
                        "complete_task",
                    );
                }
            }
            60..68 => {
                let t = shared.tasks.lock().unwrap();
                let target = (!t.is_empty()).then(|| t[rng.below(t.len())].clone());
                drop(t);
                if let Some(t) = target {
                    tolerate(cancel(svc, &t), "cancel_task");
                }
            }
            68..72 => {
                // Time passes beyond every lease, then somebody reaps.
                shared.clock.advance_secs(shared.lease_secs + 1);
                tolerate(h.expire_leases(), "expire_leases");
            }
            72..75 => {
                tolerate(h.expire_leases(), "expire_leases");
            }
            75..82 => {
                tolerate(svc.sessions().end(&session), "end session");
                new_session(&mut session, &mut held);
            }
            _ => {
                let t = h
                    .create_task(&CreateTask {
                        creator: format!("mixed-maker-{i}"),
                        title: format!("m{i}.{k}"),
                        ..CreateTask::default()
                    })
                    .unwrap();
                shared.tasks.lock().unwrap().push(t.id);
            }
        }
        if k % 10 == 0 {
            let v = violations(&conn);
            assert!(
                v.is_empty(),
                "worker {i} op {k}: invariants violated mid-run: {v:?}"
            );
        }
    }
    ops
}

#[test]
fn mixed_stress_invariants_hold_no_hang_no_busy() {
    const THREADS: usize = 16;
    const OPS: usize = 150;
    for round in 0..3 {
        let rig = Rig::new();
        let hs = rig.handles(THREADS);
        let shared = Arc::new(Shared {
            tasks: Mutex::new(Vec::new()),
            clock: rig.clock.clone(),
            path: rig.path.clone(),
            lease_secs: rig.settings.lease_ms / 1000,
        });
        // Seed some work so the first claims have something to race over.
        for k in 0..8 {
            shared
                .tasks
                .lock()
                .unwrap()
                .push(new_task(&hs[0], &format!("seed {k}")));
        }
        for h in &hs {
            h.db().reset_write_stats();
        }
        let sh = shared.clone();
        let done = race_for(&hs, Duration::from_secs(120), move |i, svc| {
            mixed_worker(i, svc, &sh, OPS)
        });
        assert_eq!(done.iter().sum::<usize>(), THREADS * OPS);

        assert_invariants(&rig, &format!("at the end of mixed round {round}"));
        // Every state change is audited exactly once.
        let eq = |a: &str, b: &str, what: &str| {
            assert_eq!(rig.n(a, &[]), rig.n(b, &[]), "round {round}: {what}");
        };
        eq(
            "SELECT count(*) FROM events WHERE kind = 'task.completed'",
            "SELECT count(*) FROM tasks WHERE status = 'done'",
            "one task.completed event per done task",
        );
        eq(
            "SELECT count(*) FROM events WHERE kind = 'task.cancelled'",
            "SELECT count(*) FROM tasks WHERE status = 'cancelled'",
            "one task.cancelled event per cancelled task",
        );
        eq(
            "SELECT count(*) FROM events WHERE kind = 'task.claimed'",
            "SELECT count(*) FROM claims",
            "one task.claimed event per claim row",
        );
        eq(
            "SELECT count(*) FROM events WHERE kind = 'lease.expired'",
            "SELECT count(*) FROM claims WHERE end_reason = 'expired'",
            "one lease.expired event per expired claim",
        );
        let (mut hold, mut wait) = (0, 0);
        for h in &hs {
            let st = h.db().write_stats();
            hold = hold.max(st.max_hold_us);
            wait = wait.max(st.max_wait_us);
        }
        eprintln!(
            "mixed round {round}: claims={} done={} cancelled={} expired={} max_hold_us={hold} max_wait_us={wait}",
            rig.n("SELECT count(*) FROM claims", &[]),
            rig.n("SELECT count(*) FROM tasks WHERE status = 'done'", &[]),
            rig.n("SELECT count(*) FROM tasks WHERE status = 'cancelled'", &[]),
            rig.n("SELECT count(*) FROM claims WHERE end_reason = 'expired'", &[]),
        );
    }
}

// ---------- 7. backstop ----------

#[test]
fn backstop_second_active_claim_by_direct_sql_is_rejected() {
    let rig = Rig::new();
    let svc = rig.handle();
    let (s1, s2) = (register(&svc, "holder"), register(&svc, "intruder"));
    let (a, b) = (new_task(&svc, "a"), new_task(&svc, "b"));
    // The service gives the first task to s1 (oldest first)...
    let won = claim(&svc, &s1).unwrap();
    let held = won.task.unwrap().id;
    assert_eq!(held, a);

    let insert = |id: &str, task: &str, session: &str| {
        exec(
            svc.db(),
            &format!(
                "INSERT INTO claims (id, task_id, session_id, agent_name, claimed_at, lease_expires_at)
                 VALUES ('{id}', '{task}', '{session}', 'intruder', 1, 2)"
            ),
        )
    };
    // ...a second active claim on the same task is rejected by the unique index,
    let e = insert("x1", &a, &s2).unwrap_err();
    assert!(
        matches!(e, CorvusError::Conflict(ref m) if m.contains("UNIQUE")),
        "{e:?}"
    );
    // as is a second active claim for the same session.
    let e = insert("x2", &b, &s1).unwrap_err();
    assert!(
        matches!(e, CorvusError::Conflict(ref m) if m.contains("UNIQUE")),
        "{e:?}"
    );

    assert_eq!(rig.n("SELECT count(*) FROM claims", &[]), 1);
    assert_eq!(
        rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL", &[]),
        1
    );
    assert_invariants(&rig, "after rejected inserts");
}

// ---------- 8. concurrent idempotency ----------

#[test]
fn idempotent_create_task_concurrent_one_task_one_event() {
    const N: usize = 16;
    let rig = Rig::new();
    let hs = rig.handles(N);
    for iter in 0..25 {
        let key = format!("create-key-{iter}");
        let k = key.clone();
        let results = race(&hs, move |_, svc| {
            svc.huginn().create_task(&CreateTask {
                creator: "idem-creator".into(),
                title: "same task".into(),
                idempotency_key: Some(k.clone()),
                ..CreateTask::default()
            })
        });
        let mut ids = HashSet::new();
        for r in &results {
            never_busy(r, "create_task");
            ids.insert(r.as_ref().unwrap().id.clone());
        }
        assert_eq!(ids.len(), 1, "iter {iter}: every caller got the same task id");
        let id = ids.into_iter().next().unwrap();
        assert_eq!(
            rig.n("SELECT count(*) FROM tasks WHERE creator = 'idem-creator'", &[]),
            iter + 1
        );
        assert_eq!(rig.n("SELECT count(*) FROM tasks WHERE id = ?1", &[&id]), 1);
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM events WHERE kind = 'task.created' AND task_id = ?1",
                &[&id]
            ),
            1,
            "iter {iter}: one task.created event"
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM threads WHERE task_id = ?1", &[&id]),
            1
        );
    }
    // The key is scoped by creator: another creator with the same key makes its own task.
    let other = hs[0]
        .huginn()
        .create_task(&CreateTask {
            creator: "someone-else".into(),
            title: "same task".into(),
            idempotency_key: Some("create-key-0".into()),
            ..CreateTask::default()
        })
        .unwrap();
    assert_eq!(rig.n("SELECT count(*) FROM tasks WHERE id = ?1", &[&other.id]), 1);
    assert_eq!(rig.n("SELECT count(*) FROM tasks", &[]), 26);
}

#[test]
fn idempotent_register_concurrent_one_session_one_event() {
    const N: usize = 16;
    let rig = Rig::new();
    let hs = rig.handles(N);
    for iter in 0..25 {
        // Named agent.
        let key = format!("reg-key-{iter}");
        let agent = format!("idem-agent-{}", iter % 3);
        let (k, a) = (key.clone(), agent.clone());
        let results = race(&hs, move |_, svc| {
            svc.sessions().register(&RegisterSession {
                agent_name: Some(a.clone()),
                client_name: "races".into(),
                idempotency_key: Some(k.clone()),
                ..RegisterSession::default()
            })
        });
        let mut ids = HashSet::new();
        for r in &results {
            never_busy(r, "register");
            ids.insert(r.as_ref().unwrap().id.clone());
        }
        assert_eq!(ids.len(), 1, "iter {iter}: every caller got the same session");
        let id = ids.into_iter().next().unwrap();
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM sessions WHERE agent_name = ?1 AND idempotency_key = ?2",
                &[&agent, &key]
            ),
            1
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM events WHERE kind = 'session.registered' AND session_id = ?1",
                &[&id]
            ),
            1,
            "iter {iter}: one session.registered event"
        );

        // Generated name with a key: replays map to the one session (and its one generated name).
        let gkey = format!("gen-key-{iter}");
        let gk = gkey.clone();
        let results = race(&hs, move |_, svc| {
            svc.sessions().register(&RegisterSession {
                agent_name: None,
                client_name: "races".into(),
                idempotency_key: Some(gk.clone()),
                ..RegisterSession::default()
            })
        });
        let mut ids = HashSet::new();
        for r in &results {
            never_busy(r, "register (generated)");
            let v = r.as_ref().unwrap();
            assert!(v.name_generated);
            ids.insert(v.id.clone());
        }
        assert_eq!(
            ids.len(),
            1,
            "iter {iter}: generated-name replays return one session"
        );
        let id = ids.into_iter().next().unwrap();
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM sessions WHERE idempotency_key = ?1 AND name_generated = 1",
                &[&gkey]
            ),
            1
        );
        assert_eq!(
            rig.n(
                "SELECT count(*) FROM events WHERE kind = 'session.registered' AND session_id = ?1",
                &[&id]
            ),
            1
        );
    }
}

// ---------- 9. generated names ----------

#[test]
fn generated_names_unique_under_concurrent_registration() {
    const N: usize = 32;
    let rig = Rig::new();
    let hs = rig.handles(N);
    let mut all = HashSet::new();
    for iter in 0..15 {
        let results = race(&hs, |_, svc| {
            svc.sessions().register(&RegisterSession {
                agent_name: None,
                client_name: "races".into(),
                ..RegisterSession::default()
            })
        });
        let mut names = HashSet::new();
        for r in &results {
            never_busy(r, "register");
            let v = r.as_ref().unwrap();
            assert!(v.name_generated, "name_generated must be true");
            assert!(
                names.insert(v.agent_name.clone()),
                "iter {iter}: duplicate name {}",
                v.agent_name
            );
            assert!(
                all.insert(v.agent_name.clone()),
                "iter {iter}: name {} reused",
                v.agent_name
            );
        }
        assert_eq!(names.len(), N);
    }
    let total = (N * 15) as i64;
    assert_eq!(
        rig.n("SELECT count(*) FROM sessions WHERE name_generated = 1", &[]),
        total
    );
    assert_eq!(
        rig.n("SELECT count(DISTINCT agent_name) FROM sessions", &[]),
        total
    );
    assert_eq!(rig.n("SELECT count(*) FROM agents", &[]), total);
    assert_eq!(
        rig.n(
            "SELECT count(*) FROM events WHERE kind = 'session.registered'",
            &[]
        ),
        total
    );
}

// ---------- 10. auto-end versus heartbeat ----------

#[test]
fn auto_end_vs_heartbeat_threads_serial_outcomes_only() {
    const HEARTBEATERS: usize = 8;
    const REAPERS: usize = 8;
    let rig = Rig::with(Settings {
        auto_end_ms: 10_000,
        ..Settings::default()
    });
    let hs = rig.handles(HEARTBEATERS + REAPERS);
    let (mut survived, mut auto_ended) = (0, 0);
    for iter in 0..120 {
        let sid = register(&hs[0], &format!("sleeper-{iter}"));
        new_task(&hs[0], &format!("held {iter}"));
        let held = claim(&hs[0], &sid).unwrap();
        let task = held.task.expect("a task to hold").id;
        // The session falls silent for longer than the auto-end threshold.
        rig.clock.advance_secs(20);
        let now = rig.clock.now_ms();

        let s = sid.clone();
        let results = race(&hs, move |i, svc| {
            if i < HEARTBEATERS {
                (Some(svc.sessions().heartbeat(&s)), None)
            } else {
                (None, Some(svc.sessions().end_stale()))
            }
        });

        let mut heartbeats = Vec::new();
        let mut ended_by_reapers = 0;
        for (hb, es) in &results {
            if let Some(hb) = hb {
                never_busy(hb, "heartbeat");
                heartbeats.push(hb);
            }
            if let Some(es) = es {
                never_busy(es, "end_stale");
                let ended = &es.as_ref().unwrap().ended;
                ended_by_reapers += ended.iter().filter(|e| **e == sid).count();
                assert!(
                    ended.iter().all(|e| *e == sid),
                    "iter {iter}: end_stale ended a session other than the silent one: {ended:?}"
                );
            }
        }
        let ended_at = rig.n(
            "SELECT coalesce(ended_at, 0) FROM sessions WHERE id = ?1",
            &[&sid],
        );
        let last_seen = rig.n("SELECT last_seen_at FROM sessions WHERE id = ?1", &[&sid]);
        assert!(
            ended_at == 0 || last_seen <= ended_at,
            "iter {iter}: ended with last_seen_at after ended_at"
        );
        let claim_end = rig.text("SELECT end_reason FROM claims WHERE session_id = ?1", &[&sid]);
        let status = rig
            .text("SELECT status FROM tasks WHERE id = ?1", &[&task])
            .unwrap();
        let auto_events = rig.n(
            "SELECT count(*) FROM events WHERE kind = 'session.auto_ended' AND session_id = ?1",
            &[&sid],
        );

        if ended_at == 0 {
            // The heartbeat landed first: the session survives with its claim.
            survived += 1;
            assert!(
                heartbeats.iter().all(|h| h.is_ok()),
                "iter {iter}: heartbeat must succeed"
            );
            assert_eq!(last_seen, now, "iter {iter}: last_seen_at == now");
            assert_eq!(ended_by_reapers, 0);
            assert_eq!(auto_events, 0);
            assert_eq!(
                claim_end, None,
                "iter {iter}: surviving session must keep its claim active"
            );
            assert_eq!(status, "claimed");
            assert_eq!(
                rig.n(
                    "SELECT count(*) FROM claims WHERE session_id = ?1 AND released_at IS NULL",
                    &[&sid]
                ),
                1
            );
        } else {
            // Auto-end landed first: heartbeats are refused, the claim is released, the task reopens.
            auto_ended += 1;
            assert!(
                heartbeats
                    .iter()
                    .all(|h| matches!(h, Err(CorvusError::SessionNotActive))),
                "iter {iter}: heartbeat after auto-end must be SessionNotActive, got {heartbeats:?}"
            );
            assert_eq!(ended_by_reapers, 1, "iter {iter}: exactly one reaper ends it");
            assert_eq!(auto_events, 1);
            assert_eq!(ended_at, now);
            assert_eq!(
                rig.text("SELECT end_reason FROM sessions WHERE id = ?1", &[&sid])
                    .unwrap(),
                "auto_ended"
            );
            assert_eq!(
                claim_end.as_deref(),
                Some("released"),
                "iter {iter}: claim ended 'released'"
            );
            assert_eq!(status, "open", "iter {iter}: task reopened");
        }
        assert_invariants(&rig, "after auto-end race");
        // Clean up so the next iteration starts from nothing held.
        cancel(&hs[0], &task).unwrap();
        hs[0].sessions().end(&sid).unwrap();
    }
    eprintln!("auto_end_vs_heartbeat: survived={survived} auto_ended={auto_ended}");
}

#[test]
fn auto_end_disabled_end_stale_ends_nothing() {
    let rig = Rig::with(Settings {
        auto_end_ms: 0,
        ..Settings::default()
    });
    let hs = rig.handles(16);
    let sid = register(&hs[0], "forever");
    let task = new_task(&hs[0], "held forever");
    assert!(claim(&hs[0], &sid).unwrap().task.is_some());
    rig.clock.advance_secs(365 * 24 * 3600);
    let results = race(&hs, |_, svc| svc.sessions().end_stale());
    for r in &results {
        never_busy(r, "end_stale");
        assert!(r.as_ref().unwrap().ended.is_empty());
    }
    assert_eq!(
        rig.n("SELECT count(*) FROM sessions WHERE ended_at IS NOT NULL", &[]),
        0
    );
    assert_eq!(
        rig.n(
            "SELECT count(*) FROM events WHERE kind = 'session.auto_ended'",
            &[]
        ),
        0
    );
    assert_eq!(
        rig.text("SELECT status FROM tasks WHERE id = ?1", &[&task])
            .unwrap(),
        "claimed"
    );
    assert_eq!(
        rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL", &[]),
        1
    );
}
