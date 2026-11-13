//! Plan 0001 verification: pragmas on every connection; foreign keys enforced; read/write separation.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::ReadOnlyDatabase;
use corvus_core::db::{Reader, SqlResultExt, pragma_snapshot};
use corvus_core::testing::{TestDb, exec};
use corvus_domain::CorvusError;

fn get(snap: &[(String, String)], k: &str) -> String {
    snap.iter().find(|(p, _)| p == k).map(|(_, v)| v.clone()).unwrap()
}

#[test]
fn writer_and_reader_connections_have_the_pragmas() {
    let t = TestDb::new();
    let w = t.db.write(|tx| pragma_snapshot(tx)).unwrap();
    let r = t.db.read(|tx| pragma_snapshot(tx)).unwrap();
    for snap in [&w, &r] {
        assert_eq!(get(snap, "journal_mode"), "wal");
        assert_eq!(get(snap, "foreign_keys"), "1");
        assert_eq!(get(snap, "synchronous"), "2");
        assert_eq!(get(snap, "busy_timeout"), "5000");
        assert_eq!(get(snap, "trusted_schema"), "0");
    }
    assert_eq!(get(&w, "query_only"), "0");
    assert_eq!(get(&r, "query_only"), "1");
}

#[test]
fn read_only_open_has_pragmas_and_cannot_write() {
    let t = TestDb::new();
    let ro = ReadOnlyDatabase::open(&t.path, 300).unwrap();
    let s = ro.read(|tx| pragma_snapshot(tx)).unwrap();
    assert_eq!(get(&s, "journal_mode"), "wal");
    assert_eq!(get(&s, "foreign_keys"), "1");
    assert_eq!(get(&s, "busy_timeout"), "300");
    assert_eq!(get(&s, "query_only"), "1");
    let e = ro.read(|tx| {
        tx.sql()
            .execute("INSERT INTO agents (name, created_at) VALUES ('x', 1)", [])
            .db()
    });
    assert!(e.is_err());
}

#[test]
fn read_only_open_refuses_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("none.db");
    assert!(ReadOnlyDatabase::open(&p, 300).is_err());
    assert!(!p.exists(), "the read-only opener must not create the file");
}

#[test]
fn reader_transaction_cannot_write_even_through_raw_sql() {
    let t = TestDb::new();
    let e = t.db.read(|tx| {
        tx.sql()
            .execute("INSERT INTO agents (name, created_at) VALUES ('x', 1)", [])
            .db()
    });
    assert!(e.is_err(), "query_only must reject writes");
}

#[test]
fn foreign_key_violation_is_rejected() {
    let t = TestDb::new();
    let e = exec(
        &t.db,
        "INSERT INTO sessions (id, agent_name, client_name, started_at, last_seen_at)
         VALUES ('s1', 'nobody', 'c', 1, 1)",
    )
    .unwrap_err();
    assert!(
        matches!(e, CorvusError::Conflict(ref m) if m.contains("FOREIGN KEY")),
        "{e:?}"
    );
}

#[test]
fn failed_write_closure_rolls_back() {
    let t = TestDb::new();
    let r: Result<(), CorvusError> = t.db.write(|tx| {
        tx.sql_mut()
            .execute("INSERT INTO agents (name, created_at) VALUES ('rolled', 1)", [])
            .db()?;
        Err(CorvusError::Conflict("stop".into()))
    });
    assert!(r.is_err());
    assert_eq!(corvus_core::testing::count(&t.db, "agents"), 0);
}
