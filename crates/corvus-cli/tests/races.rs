//! Plan 0002 verification: the same races as `corvus-core/tests/races.rs`, between real `corvus` processes
//! on one database file ([technique](../../../spec/system/09-testing.md#concurrency-test-technique)).
//!
//! Starting N processes "together" without changing the product: the test holds SQLite's write lock (a
//! `BEGIN IMMEDIATE` transaction) while it spawns the children. Every child's single command blocks on the
//! busy handler at its first write and they are all released together when the test commits. The database
//! is migrated and seeded before that, so the children only run their one command.
//!
//! Processes use the system clock, so these tests seed with a `ManualClock` set to the real time (or, for
//! auto-end, to the past) rather than moving time forward.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::collections::HashSet;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use common::{Home, json_stdout, stderr};
use corvus_core::api::{ClaimTask, CreateTask, RegisterSession};
use corvus_core::testing::query_i64;
use corvus_core::{Clock, Database, ManualClock, Services, Settings, SystemClock};

/// How long the test holds the write lock while the children are spawned.
const HOLD: Duration = Duration::from_millis(900);

/// A migrated database in a throwaway `HOME`, with a service handle for seeding and assertions.
struct Rig {
    home: Home,
    svc: Services,
    clock: Arc<ManualClock>,
}

impl Rig {
    /// `start_ms`: where the seeding clock stands (the system time, or earlier).
    fn new(start_ms: i64) -> Self {
        let home = Home::new();
        json_stdout(&home.corvus().arg("migrate").output().unwrap());
        let clock = Arc::new(ManualClock::new(start_ms));
        let (db, _) = Database::open(&home.db_path(), clock.as_ref()).unwrap();
        let svc = Services::new(Arc::new(db), clock.clone(), Settings::default());
        Self { home, svc, clock }
    }

    fn now() -> Self {
        Self::new(SystemClock.now_ms())
    }

    fn corvus(&self) -> Command {
        self.home.corvus()
    }

    fn db(&self) -> &Database {
        self.svc.db()
    }

    fn n(&self, sql: &str) -> i64 {
        query_i64(self.db(), sql).unwrap()
    }

    fn session(&self, agent: &str) -> String {
        self.svc
            .sessions()
            .register(&RegisterSession {
                agent_name: Some(agent.to_owned()),
                client_name: "races".into(),
                ..RegisterSession::default()
            })
            .unwrap()
            .id
    }

    fn task(&self, title: &str) -> String {
        self.svc
            .huginn()
            .create_task(&CreateTask {
                creator: "operator".into(),
                title: title.into(),
                ..CreateTask::default()
            })
            .unwrap()
            .id
    }

    /// Runs the commands "together" (see the module docs) and returns their outputs in order.
    fn together(&self, cmds: Vec<Command>) -> Vec<Output> {
        let path = self.home.db_path();
        let (db, _) = Database::open(&path, self.clock.as_ref()).unwrap();
        let (held_tx, held_rx) = mpsc::channel();
        let locker = std::thread::spawn(move || {
            db.write(|_tx| {
                let _ = held_tx.send(());
                std::thread::sleep(HOLD);
                Ok(())
            })
            .unwrap();
        });
        held_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the lock was taken");
        let children: Vec<Child> = cmds
            .into_iter()
            .map(|mut c| c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
            .collect();
        locker.join().unwrap();
        children
            .into_iter()
            .map(|c| {
                // A hung child fails the test via the harness timeout of wait_timeout-free polling below.
                wait_with_deadline(c, Duration::from_secs(60))
            })
            .collect()
    }
}

/// Waits for a child, killing it and failing if it takes longer than `limit` (a hang fails the test).
fn wait_with_deadline(mut child: Child, limit: Duration) -> Output {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().unwrap() {
            Some(_) => return child.wait_with_output().unwrap(),
            None if start.elapsed() > limit => {
                let _ = child.kill();
                panic!("child process hung for more than {limit:?}");
            }
            None => std::thread::sleep(Duration::from_millis(5)), // polling a child, not waiting for time
        }
    }
}

/// Every process must have exited 0; never exit 6 (busy). Returns the parsed JSON of each.
fn all_ok(outs: &[Output], what: &str) -> Vec<serde_json::Value> {
    outs.iter()
        .enumerate()
        .map(|(i, o)| {
            assert_ne!(
                o.status.code(),
                Some(6),
                "{what} #{i} reported busy: {}",
                stderr(o)
            );
            json_stdout(o)
        })
        .collect()
}

fn claim_cmd(rig: &Rig, session: &str) -> Command {
    let mut c = rig.corvus();
    c.args(["task", "claim", "--session", session]);
    c
}

fn cancel_cmd(rig: &Rig, task: &str) -> Command {
    let mut c = rig.corvus();
    c.args(["task", "cancel", task, "--by", "operator", "--reason", "race"]);
    c
}

/// Structural invariants of the work queue; returns the number of violations.
fn violations(rig: &Rig) -> i64 {
    [
        "SELECT count(*) FROM (SELECT 1 FROM claims WHERE released_at IS NULL GROUP BY task_id HAVING count(*) > 1)",
        "SELECT count(*) FROM (SELECT 1 FROM claims WHERE released_at IS NULL GROUP BY session_id HAVING count(*) > 1)",
        "SELECT count(*) FROM tasks t WHERE t.status = 'claimed'
           AND (SELECT count(*) FROM claims c WHERE c.task_id = t.id AND c.released_at IS NULL) <> 1",
        "SELECT count(*) FROM claims c JOIN tasks t ON t.id = c.task_id
           WHERE c.released_at IS NULL AND t.status <> 'claimed'",
        "SELECT count(*) FROM claims c JOIN sessions s ON s.id = c.session_id
           WHERE c.released_at IS NULL AND s.ended_at IS NOT NULL",
    ]
    .iter()
    .map(|q| rig.n(q))
    .sum()
}

// ---------- 2. claim race (processes) ----------

#[test]
fn claim_race_processes_exactly_one_winner() {
    const N: usize = 16;
    for round in 0..3 {
        let rig = Rig::now();
        let task = rig.task("contested");
        let sessions: Vec<String> = (0..N).map(|i| rig.session(&format!("proc-{i}"))).collect();
        let outs = rig.together(sessions.iter().map(|s| claim_cmd(&rig, s)).collect());
        let vals = all_ok(&outs, "claim");

        let winners: Vec<usize> = vals
            .iter()
            .enumerate()
            .filter(|(_, v)| !v["task"].is_null())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            winners.len(),
            1,
            "round {round}: exactly one winner, got {winners:?}"
        );
        for (i, v) in vals.iter().enumerate() {
            if i == winners[0] {
                assert_eq!(v["task"]["id"], task.as_str());
                assert_eq!(v["claim"]["session_id"], sessions[i].as_str());
            } else {
                assert!(v["task"].is_null() && v["claim"].is_null(), "loser {i}: {v}");
            }
        }
        // Database state.
        assert_eq!(rig.n("SELECT count(*) FROM claims"), 1);
        assert_eq!(
            rig.n(&format!(
                "SELECT count(*) FROM claims WHERE task_id = '{task}' AND released_at IS NULL AND session_id = '{}'",
                sessions[winners[0]]
            )),
            1
        );
        assert_eq!(
            rig.n(&format!(
                "SELECT count(*) FROM tasks WHERE id = '{task}' AND status = 'claimed'"
            )),
            1
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM events WHERE kind = 'task.claimed'"),
            1
        );
        assert_eq!(violations(&rig), 0);
    }
}

#[test]
fn claim_fan_out_processes_n_claimers_n_tasks_n_distinct_tasks() {
    const N: usize = 12;
    for round in 0..2 {
        let rig = Rig::now();
        let tasks: HashSet<String> = (0..N).map(|i| rig.task(&format!("fan {i}"))).collect();
        let sessions: Vec<String> = (0..N).map(|i| rig.session(&format!("fan-{i}"))).collect();
        let outs = rig.together(sessions.iter().map(|s| claim_cmd(&rig, s)).collect());
        let vals = all_ok(&outs, "claim");
        let got: HashSet<String> = vals
            .iter()
            .map(|v| {
                v["task"]["id"]
                    .as_str()
                    .expect("every claimer got a task")
                    .to_owned()
            })
            .collect();
        assert_eq!(got, tasks, "round {round}: N distinct tasks, one per claimer");
        assert_eq!(
            rig.n("SELECT count(*) FROM claims WHERE released_at IS NULL"),
            N as i64
        );
        assert_eq!(
            rig.n("SELECT count(DISTINCT task_id) FROM claims WHERE released_at IS NULL"),
            N as i64
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM tasks WHERE status = 'claimed'"),
            N as i64
        );
        assert_eq!(
            rig.n("SELECT count(*) FROM events WHERE kind = 'task.claimed'"),
            N as i64
        );
        assert_eq!(violations(&rig), 0);
    }
}

// ---------- 5. cancel versus claim (processes) ----------

#[test]
fn cancel_vs_claim_processes_only_serial_outcomes() {
    const CLAIMERS: usize = 6;
    const CANCELLERS: usize = 6;
    let (mut claimed_then_cancelled, mut cancelled_first) = (0, 0);
    for round in 0..8 {
        let rig = Rig::now();
        let task = rig.task("doomed");
        let sessions: Vec<String> = (0..CLAIMERS).map(|i| rig.session(&format!("cvc-{i}"))).collect();
        let mut cmds: Vec<Command> = sessions.iter().map(|s| claim_cmd(&rig, s)).collect();
        cmds.extend((0..CANCELLERS).map(|_| cancel_cmd(&rig, &task)));
        let vals = all_ok(&rig.together(cmds), "claim/cancel");

        let winners: Vec<usize> = (0..CLAIMERS).filter(|&i| !vals[i]["task"].is_null()).collect();
        assert!(winners.len() <= 1, "round {round}: two winners {winners:?}");
        let effective = vals[CLAIMERS..]
            .iter()
            .filter(|v| !v["cancelled"].as_array().unwrap().is_empty())
            .count();
        assert_eq!(effective, 1, "round {round}: exactly one cancel takes effect");

        assert_eq!(
            rig.n(&format!(
                "SELECT count(*) FROM tasks WHERE id = '{task}' AND status = 'cancelled'"
            )),
            1
        );
        assert_eq!(
            rig.n(&format!(
                "SELECT count(*) FROM claims WHERE task_id = '{task}' AND released_at IS NULL"
            )),
            0,
            "round {round}: live claim on a cancelled task"
        );
        let status_msgs = format!(
            "SELECT count(*) FROM messages m JOIN threads th ON th.id = m.thread_id
             WHERE th.task_id = '{task}' AND m.type = 'status'"
        );
        match winners.first() {
            Some(&w) => {
                claimed_then_cancelled += 1;
                assert_eq!(
                    rig.n(&format!("SELECT count(*) FROM claims WHERE task_id = '{task}'")),
                    1
                );
                assert_eq!(
                    rig.n(&format!(
                        "SELECT count(*) FROM claims WHERE task_id = '{task}' AND session_id = '{}'
                         AND end_reason = 'cancelled' AND released_at IS NOT NULL",
                        sessions[w]
                    )),
                    1,
                    "round {round}: claim must end 'cancelled'"
                );
                assert_eq!(rig.n(&format!("{status_msgs} AND m.recipient = 'cvc-{w}'")), 1);
                assert_eq!(rig.n(&status_msgs), 1);
            }
            None => {
                cancelled_first += 1;
                assert_eq!(
                    rig.n(&format!("SELECT count(*) FROM claims WHERE task_id = '{task}'")),
                    0
                );
                assert_eq!(rig.n(&status_msgs), 0);
            }
        }
        assert_eq!(
            rig.n(&format!(
                "SELECT count(*) FROM events WHERE kind = 'task.cancelled' AND task_id = '{task}'"
            )),
            1
        );
        assert_eq!(violations(&rig), 0);
    }
    eprintln!(
        "cancel_vs_claim_processes: claimed-then-cancelled={claimed_then_cancelled} cancelled-first={cancelled_first}"
    );
}

// ---------- 10. auto-end versus heartbeat (processes) ----------

/// Processes use the system clock and the test may not sleep, so the session is seeded with a manual clock an
/// hour in the past and the children run with `CORVUS_AUTO_END_SECONDS=60`: the session is already silent
/// for longer than the threshold when the race starts, and a heartbeat (system "now") rescues it.
#[test]
fn auto_end_vs_heartbeat_processes_serial_outcomes_only() {
    const HEARTBEATERS: usize = 6;
    const REAPERS: usize = 6;
    let (mut survived, mut auto_ended) = (0, 0);
    for round in 0..8 {
        let started = SystemClock.now_ms();
        let rig = Rig::new(started - 3_600_000);
        let sid = rig.session("sleeper");
        rig.task("held");
        let held = rig
            .svc
            .huginn()
            .claim_task(&ClaimTask {
                session_id: sid.clone(),
                pool: None,
            })
            .unwrap();
        let task = held.task.unwrap().id;

        let mut cmds = Vec::new();
        for _ in 0..HEARTBEATERS {
            let mut c = rig.corvus();
            c.env("CORVUS_AUTO_END_SECONDS", "60")
                .args(["session", "heartbeat", &sid]);
            cmds.push(c);
        }
        for _ in 0..REAPERS {
            let mut c = rig.corvus();
            c.env("CORVUS_AUTO_END_SECONDS", "60")
                .args(["admin", "end-stale"]);
            cmds.push(c);
        }
        let outs = rig.together(cmds);

        let mut reaped = 0;
        for (i, o) in outs.iter().enumerate() {
            assert_ne!(
                o.status.code(),
                Some(6),
                "process {i} reported busy: {}",
                stderr(o)
            );
            if i >= HEARTBEATERS {
                let v = json_stdout(o);
                reaped += v["ended"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| **e == sid.as_str())
                    .count();
            }
        }
        let hb_codes: Vec<Option<i32>> = outs[..HEARTBEATERS].iter().map(|o| o.status.code()).collect();
        let ended = rig.n(&format!(
            "SELECT count(*) FROM sessions WHERE id = '{sid}' AND ended_at IS NOT NULL"
        ));
        assert_eq!(
            rig.n("SELECT count(*) FROM sessions WHERE ended_at IS NOT NULL AND last_seen_at > ended_at"),
            0
        );
        let status_is = |s: &str| {
            rig.n(&format!(
                "SELECT count(*) FROM tasks WHERE id = '{task}' AND status = '{s}'"
            ))
        };
        let auto_events = rig.n(&format!(
            "SELECT count(*) FROM events WHERE kind = 'session.auto_ended' AND session_id = '{sid}'"
        ));

        if ended == 0 {
            survived += 1;
            assert!(
                hb_codes.iter().all(|c| *c == Some(0)),
                "round {round}: heartbeats {hb_codes:?}"
            );
            assert_eq!(reaped, 0);
            assert_eq!(auto_events, 0);
            assert!(
                rig.n(&format!("SELECT last_seen_at FROM sessions WHERE id = '{sid}'")) >= started,
                "heartbeat moved last_seen_at"
            );
            assert_eq!(
                rig.n(&format!(
                    "SELECT count(*) FROM claims WHERE session_id = '{sid}' AND released_at IS NULL"
                )),
                1
            );
            assert_eq!(status_is("claimed"), 1);
        } else {
            auto_ended += 1;
            assert!(
                hb_codes.iter().all(|c| *c == Some(5)),
                "round {round}: heartbeats {hb_codes:?}"
            );
            assert_eq!(reaped, 1, "round {round}: exactly one reaper ends the session");
            assert_eq!(auto_events, 1);
            assert_eq!(
                rig.n(&format!(
                    "SELECT count(*) FROM sessions WHERE id = '{sid}' AND end_reason = 'auto_ended'"
                )),
                1
            );
            assert_eq!(
                rig.n(&format!(
                    "SELECT count(*) FROM claims WHERE session_id = '{sid}' AND end_reason = 'released'"
                )),
                1,
                "round {round}: claim ended 'released'"
            );
            assert_eq!(status_is("open"), 1);
        }
        assert_eq!(violations(&rig), 0);
    }
    eprintln!("auto_end_vs_heartbeat_processes: survived={survived} auto_ended={auto_ended}");
}
