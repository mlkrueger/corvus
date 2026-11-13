//! Plan 0001 verification: UPDATE, DELETE … WHERE and bare DELETE fail on messages, digests and events (P4).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::testing::{TestDb, count, exec};

fn seeded() -> TestDb {
    let t = TestDb::new();
    t.agent("alice");
    t.agent("bob");
    let n = t.now();
    exec(
        &t.db,
        &format!("INSERT INTO threads (id, title, creator, created_at) VALUES ('th1', 't', 'alice', {n})"),
    )
    .unwrap();
    exec(
        &t.db,
        &format!(
            "INSERT INTO messages (thread_id, sender, recipient, type, body, created_at)
             VALUES ('th1', 'alice', 'bob', 'note', 'hello', {n})"
        ),
    )
    .unwrap();
    exec(
        &t.db,
        &format!(
            "INSERT INTO digests (scope, summary, author, created_at) VALUES ('global', 's', 'alice', {n})"
        ),
    )
    .unwrap();
    exec(
        &t.db,
        &format!("INSERT INTO events (kind, summary, created_at) VALUES ('task.created', 's', {n})"),
    )
    .unwrap();
    t
}

#[test]
fn append_only_tables_reject_update_and_delete() {
    let t = seeded();
    let cases = [
        ("messages", "UPDATE messages SET body = 'changed'"),
        ("messages", "DELETE FROM messages WHERE id = 1"),
        ("messages", "DELETE FROM messages"),
        ("digests", "UPDATE digests SET summary = 'changed'"),
        ("digests", "DELETE FROM digests WHERE id = 1"),
        ("digests", "DELETE FROM digests"),
        ("events", "UPDATE events SET summary = 'changed'"),
        ("events", "DELETE FROM events WHERE id = 1"),
        ("events", "DELETE FROM events"),
    ];
    for (table, sql) in cases {
        let e = exec(&t.db, sql).unwrap_err();
        assert!(e.to_string().contains("append-only table"), "{sql}: {e}");
        assert_eq!(count(&t.db, table), 1, "{sql} must change nothing");
    }
}

#[test]
fn other_tables_still_allow_updates() {
    let t = seeded();
    assert_eq!(exec(&t.db, "UPDATE threads SET status = 'closed'").unwrap(), 1);
}
