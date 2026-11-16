//! Plan 0001 verification: migrations ordered, checksummed, re-runnable, newer database refused, backup.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::db::migrator::{self, SCHEMA_VERSION, backup_path};
use corvus_core::db::{Database, OpenError};
use corvus_core::testing::{TestDb, count, exec, mode, query_i64};
use corvus_core::{Clock, ManualClock};

#[test]
fn applies_to_an_empty_database_and_second_run_is_a_noop() {
    let t = TestDb::new();
    assert_eq!(t.report.from_version, 0);
    assert_eq!(t.report.to_version, SCHEMA_VERSION);
    assert_eq!(t.report.applied.len(), 7);
    assert_eq!(t.report.applied[0], "0001_agents_sessions");
    assert_eq!(t.report.backup, None, "an empty database is not backed up");
    let applied = t.db.read(|tx| migrator::applied(tx)).unwrap();
    assert_eq!(
        applied.iter().map(|m| m.version).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7]
    );
    assert!(applied.iter().all(|m| m.applied_at == t.now()));
    assert_eq!(query_i64(&t.db, "PRAGMA user_version"), Some(SCHEMA_VERSION));

    let (_db2, again) = Database::open(&t.path, t.clock.as_ref()).unwrap();
    assert!(again.applied.is_empty());
    assert_eq!((again.from_version, again.to_version), (7, 7));
    assert_eq!(count(&t.db, "schema_migrations"), 7);
}

#[test]
fn creates_private_directory_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep/state/corvus.db");
    let _ = Database::open(&path, &ManualClock::default()).unwrap();
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn edited_applied_migration_fails_with_checksum_error_and_applies_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corvus.db");
    let clock = ManualClock::default();
    let all = migrator::embedded();
    {
        let (_db, r) = Database::open_with_migrations(&path, &all[..3], &clock).unwrap();
        assert_eq!(r.to_version, 3);
    }
    let mut edited = all.clone();
    edited[1].sql.push_str("\n-- harmless looking edit\n");
    let err = Database::open_with_migrations(&path, &edited, &clock).unwrap_err();
    match &err {
        OpenError::Checksum { version, name } => assert_eq!((*version, name.as_str()), (2, "tasks_claims")),
        other => panic!("expected checksum error, got {other}"),
    }
    assert!(err.to_string().contains("checksum"));

    // Nothing applied: still at v3, and 0004's tables do not exist.
    let (db, r) = Database::open_with_migrations(&path, &all[..3], &clock).unwrap();
    assert!(r.applied.is_empty());
    assert_eq!(query_i64(&db, "PRAGMA user_version"), Some(3));
    assert_eq!(
        query_i64(&db, "SELECT count(*) FROM sqlite_schema WHERE name = 'events'"),
        Some(0)
    );
    assert_eq!(count(&db, "schema_migrations"), 3);
}

#[test]
fn newer_database_is_refused() {
    let t = TestDb::new();
    exec(&t.db, "PRAGMA user_version = 99").unwrap();
    let err = Database::open(&t.path, t.clock.as_ref()).unwrap_err();
    assert!(
        matches!(
            err,
            OpenError::Newer {
                database: 99,
                binary: 7
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("upgrade Corvus"), "{err}");
}

#[test]
fn running_process_detects_version_skew_on_write() {
    let t = TestDb::new();
    let other = t.reopen();
    // Simulates another process upgrading the schema underneath this one.
    exec(&other, "PRAGMA user_version = 99").unwrap();
    assert_eq!(query_i64(&t.db, "PRAGMA user_version"), Some(99));
    let err = exec(&t.db, "INSERT INTO agents (name, created_at) VALUES ('a', 1)").unwrap_err();
    assert!(err.to_string().contains("restart this process"), "{err}");
}

#[test]
fn backup_written_before_migrating_a_non_empty_database_and_it_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corvus.db");
    let clock = ManualClock::default();
    let all = migrator::embedded();
    {
        let (db, _) = Database::open_with_migrations(&path, &all[..3], &clock).unwrap();
        exec(
            &db,
            &format!(
                "INSERT INTO agents (name, created_at) VALUES ('keeper', {})",
                clock.now_ms()
            ),
        )
        .unwrap();
    }
    let (_db, r) = Database::open(&path, &clock).unwrap();
    let bak = backup_path(&path, SCHEMA_VERSION);
    assert_eq!(r.backup.as_deref(), Some(bak.as_path()));
    assert_eq!(r.applied.len(), 4);
    assert_eq!(mode(&bak), 0o600);

    // The backup is the pre-migration database: v3 with the row in it.
    let (bdb, br) = Database::open_with_migrations(&bak, &all[..3], &clock).unwrap();
    assert!(br.applied.is_empty());
    assert_eq!(query_i64(&bdb, "PRAGMA user_version"), Some(3));
    assert_eq!(count(&bdb, "agents"), 1);
}

#[test]
fn concurrent_openers_in_threads_apply_each_migration_once() {
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corvus.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (p, b) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    Database::open(&p, &ManualClock::default()).map(|(_, r)| r.applied.len())
                })
            })
            .collect();
        let applied: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
        assert_eq!(applied.iter().sum::<usize>(), 7, "{applied:?}");
        let (db, _) = Database::open(&path, &ManualClock::default()).unwrap();
        assert_eq!(count(&db, "schema_migrations"), 7);
    }
}
