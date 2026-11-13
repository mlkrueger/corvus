//! Plan 0001 verification: the store's constraints, and agreement between SQL checks and domain validators.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::db::Database;
use corvus_core::testing::{TestDb, exec};
use corvus_domain::{AgentName, CorvusError, validate_artifact_path, validate_commit_hash};
use proptest::prelude::*;

fn claim(t: &TestDb, id: &str, task: &str, session: &str) -> Result<usize, CorvusError> {
    exec(
        &t.db,
        &format!(
            "INSERT INTO claims (id, task_id, session_id, agent_name, claimed_at, lease_expires_at)
             VALUES ('{id}', '{task}', '{session}', 'alice', 1, 2)"
        ),
    )
}

#[test]
fn second_active_claim_per_task_rejected() {
    let t = TestDb::new();
    let task = t.task("alice", "x");
    let s1 = t.session("alice");
    let s2 = t.session("alice");
    claim(&t, "c1", &task.to_string(), &s1.to_string()).unwrap();
    let e = claim(&t, "c2", &task.to_string(), &s2.to_string()).unwrap_err();
    assert!(
        matches!(e, CorvusError::Conflict(ref m) if m.contains("UNIQUE")),
        "{e:?}"
    );
    // Once released, a new claim on the same task is fine.
    exec(
        &t.db,
        "UPDATE claims SET released_at = 3, end_reason = 'released' WHERE id = 'c1'",
    )
    .unwrap();
    claim(&t, "c2", &task.to_string(), &s2.to_string()).unwrap();
}

#[test]
fn second_active_claim_per_session_rejected() {
    let t = TestDb::new();
    let a = t.task("alice", "a");
    let b = t.task("alice", "b");
    let s = t.session("alice");
    claim(&t, "c1", &a.to_string(), &s.to_string()).unwrap();
    assert!(claim(&t, "c2", &b.to_string(), &s.to_string()).is_err());
}

#[test]
fn claim_end_reason_and_released_at_go_together() {
    let t = TestDb::new();
    let task = t.task("alice", "x");
    let s = t.session("alice");
    claim(&t, "c1", &task.to_string(), &s.to_string()).unwrap();
    assert!(exec(&t.db, "UPDATE claims SET released_at = 3 WHERE id = 'c1'").is_err());
    assert!(exec(&t.db, "UPDATE claims SET end_reason = 'released' WHERE id = 'c1'").is_err());
}

#[test]
fn reply_across_threads_rejected() {
    let t = TestDb::new();
    t.agent("alice");
    for th in ["t1", "t2"] {
        exec(
            &t.db,
            &format!("INSERT INTO threads (id, title, creator, created_at) VALUES ('{th}', 't', 'alice', 1)"),
        )
        .unwrap();
    }
    let msg = |thread: &str, reply: &str| {
        exec(
            &t.db,
            &format!(
                "INSERT INTO messages (thread_id, sender, type, in_reply_to, body, created_at)
                 VALUES ('{thread}', 'alice', 'answer', {reply}, 'b', 1)"
            ),
        )
    };
    msg("t1", "NULL").unwrap(); // id 1 in t1
    msg("t1", "1").unwrap(); // same-thread reply: fine
    let e = msg("t2", "1").unwrap_err();
    assert!(e.to_string().contains("FOREIGN KEY"), "{e}");
    assert!(msg("t1", "999").is_err(), "reply to a missing message");
}

fn artifact(t: &TestDb, path: &str, commit: Option<&str>) -> Result<usize, CorvusError> {
    t.db.write(|tx| {
        tx.sql_mut()
            .execute(
                "INSERT INTO artifacts (id, kind, path, commit_hash, task_id, created_at)
                 VALUES (lower(hex(randomblob(16))), 'file', ?1, ?2, (SELECT id FROM tasks LIMIT 1), 1)",
                (path, commit),
            )
            .map_err(corvus_core::db::sql_err)
    })
}

#[test]
fn artifact_paths() {
    let t = TestDb::new();
    t.task("alice", "x");
    for bad in ["..", "../x", "a/../b", "a/..", "/etc/passwd", "a\0b", ""] {
        assert!(artifact(&t, bad, None).is_err(), "{bad:?} must be rejected");
        assert!(validate_artifact_path(bad).is_err(), "domain must reject {bad:?}");
    }
    for good in ["src/main.rs", "a..b", "..a", "a/..b/c", "feature/x"] {
        artifact(&t, good, None).unwrap_or_else(|e| panic!("{good:?}: {e}"));
    }
    assert!(artifact(&t, "f", Some("ABCDEF1")).is_err(), "uppercase commit");
    assert!(artifact(&t, "f", Some("abc")).is_err(), "short commit");
    artifact(&t, "f", Some("abcdef1")).unwrap();
}

#[test]
fn artifact_needs_exactly_one_owner() {
    let t = TestDb::new();
    t.task("alice", "x");
    let e = exec(
        &t.db,
        "INSERT INTO artifacts (id, kind, path, created_at) VALUES ('a', 'file', 'f', 1)",
    );
    assert!(e.is_err());
}

#[test]
fn titles_and_bodies() {
    let t = TestDb::new();
    t.agent("alice");
    let task = |title: &str, body: &str| {
        t.db.write(|tx| {
            tx.sql_mut()
                .execute(
                    "INSERT INTO tasks (id, title, body, creator, created_at, updated_at)
                     VALUES (lower(hex(randomblob(16))), ?1, ?2, 'alice', 1, 1)",
                    (title, body),
                )
                .map_err(corvus_core::db::sql_err)
        })
    };
    assert!(matches!(task("", ""), Err(CorvusError::Validation(_))));
    assert!(task("   ", "").is_err());
    assert!(task("t", &"x".repeat(20_001)).is_err());
    task("t", &"é".repeat(20_000)).unwrap(); // counted in characters, not bytes
    task("t", "").unwrap();

    exec(
        &t.db,
        "INSERT INTO threads (id, title, creator, created_at) VALUES ('th', 't', 'alice', 1)",
    )
    .unwrap();
    assert!(
        exec(
            &t.db,
            "INSERT INTO threads (id, title, creator, created_at) VALUES ('t2', ' ', 'alice', 1)"
        )
        .is_err()
    );
    let msg = |body: &str| {
        t.db.write(|tx| {
            tx.sql_mut()
                .execute(
                    "INSERT INTO messages (thread_id, sender, type, body, created_at) VALUES ('th', 'alice', 'note', ?1, 1)",
                    [body],
                )
                .map_err(corvus_core::db::sql_err)
        })
    };
    assert!(msg("").is_err());
    assert!(msg("  ").is_err());
    assert!(msg(&"x".repeat(20_001)).is_err());
    msg(&"x".repeat(20_000)).unwrap();
    assert!(exec(&t.db, "INSERT INTO messages (thread_id, sender, type, body, created_at) VALUES ('th', 'alice', 'shout', 'b', 1)").is_err());
}

fn sql_accepts_agent(db: &Database, name: &str) -> bool {
    db.write(|tx| {
        // Savepoint-free: insert then report; the transaction is rolled back by returning an error.
        let ok = tx
            .sql_mut()
            .execute("INSERT INTO agents (name, created_at) VALUES (?1, 1)", [name])
            .is_ok();
        Err::<(), _>(CorvusError::Conflict(if ok {
            "ok".into()
        } else {
            "rejected".into()
        }))
    })
    .unwrap_err()
        == CorvusError::Conflict("ok".into())
}

#[test]
fn agent_name_examples() {
    let t = TestDb::new();
    for good in ["a", "A1", "9lives", "x.y-z_w", &"a".repeat(64)] {
        assert!(sql_accepts_agent(&t.db, good), "{good}");
        assert!(AgentName::parse(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "-a",
        ".a",
        "_a",
        "a b",
        "a:b",
        "a/b",
        "é",
        &"a".repeat(65),
        "a\0",
    ] {
        assert!(!sql_accepts_agent(&t.db, bad), "{bad:?}");
        assert!(AgentName::parse(bad).is_err(), "{bad:?}");
    }
}

fn run_prop<S: Strategy>(
    cases: u32,
    strategy: S,
    check: impl Fn(&TestDb, S::Value) -> Result<(), TestCaseError>,
) where
    S::Value: std::fmt::Debug,
{
    let t = TestDb::new();
    t.task("alice", "x");
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(cases));
    runner.run(&strategy, |v| check(&t, v)).unwrap();
}

/// The SQL GLOB check and the Rust validator agree on arbitrary strings, valid shapes and near misses.
#[test]
fn agent_name_sql_and_domain_agree() {
    let s = prop_oneof![
        "[A-Za-z0-9][A-Za-z0-9_.-]{0,63}",
        "[A-Za-z0-9_.:/ -]{0,70}",
        "[a-z]{1,3}\\x00[a-z/.]{0,5}",
        "\\PC{0,8}",
        any::<String>(),
    ];
    run_prop(1000, s, |t, name| {
        prop_assert_eq!(
            sql_accepts_agent(&t.db, &name),
            AgentName::parse(&name).is_ok(),
            "{:?}",
            name
        );
        Ok(())
    });
}

/// Anything the domain accepts the store accepts (the domain may be stricter, never looser).
#[test]
fn artifact_path_domain_is_at_least_as_strict() {
    let s = prop_oneof![
        "[a-z./]{1,12}",
        "(\\.\\./|/|[a-z]{1,3}/){0,4}(\\.\\.|[a-z]{1,3})",
        any::<String>(),
    ];
    run_prop(1000, s, |t, path| {
        if validate_artifact_path(&path).is_ok() {
            prop_assert!(artifact(t, &path, None).is_ok(), "{:?}", path);
        }
        if artifact(t, &path, None).is_err() {
            prop_assert!(validate_artifact_path(&path).is_err(), "{:?}", path);
        }
        Ok(())
    });
}

#[test]
fn commit_hash_sql_and_domain_agree() {
    let s = prop_oneof!["[0-9a-f]{5,42}", "[0-9a-fA-Fg]{7,12}", any::<String>()];
    run_prop(1000, s, |t, h| {
        prop_assert_eq!(
            artifact(t, "f", Some(&h)).is_ok(),
            validate_commit_hash(&h).is_ok(),
            "{:?}",
            h
        );
        Ok(())
    });
}
