//! Plan 0001 verification: messages and task changes write wake-up hints, atomically, via triggers.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::db::{Reader, SqlResultExt};
use corvus_core::testing::{TestDb, exec};

fn topics(t: &TestDb) -> Vec<String> {
    t.db.read(|tx| {
        let mut s = tx.sql().prepare("SELECT topic FROM hints ORDER BY id").db()?;
        let rows = s.query_map([], |r| r.get(0)).db()?;
        rows.collect::<Result<Vec<String>, _>>().db()
    })
    .unwrap()
}

#[test]
fn message_insert_writes_inbox_hints() {
    let t = TestDb::new();
    t.agent("alice");
    t.agent("bob");
    exec(
        &t.db,
        "INSERT INTO threads (id, title, creator, created_at) VALUES ('th', 't', 'alice', 1)",
    )
    .unwrap();
    exec(&t.db, "DELETE FROM hints").unwrap();
    exec(
        &t.db,
        "INSERT INTO messages (thread_id, sender, recipient, type, body, created_at)
         VALUES ('th', 'alice', 'bob', 'note', 'x', 1)",
    )
    .unwrap();
    exec(
        &t.db,
        "INSERT INTO messages (thread_id, sender, recipient, type, body, created_at)
         VALUES ('th', 'alice', NULL, 'status', 'x', 1)",
    )
    .unwrap();
    assert_eq!(topics(&t), vec!["inbox:bob", "inbox:*"]);
}

#[test]
fn task_insert_and_update_write_tasks_hint() {
    let t = TestDb::new();
    let id = t.task("alice", "do it");
    assert_eq!(topics(&t), vec!["tasks"]);
    exec(&t.db, &format!("UPDATE tasks SET priority = 5 WHERE id = '{id}'")).unwrap();
    assert_eq!(topics(&t), vec!["tasks", "tasks"]);
}

#[test]
fn rolled_back_change_leaves_no_hint() {
    let t = TestDb::new();
    t.agent("alice");
    let r: Result<(), _> = t.db.write(|tx| {
        tx.sql_mut()
            .execute(
                "INSERT INTO tasks (id, title, creator, created_at, updated_at) VALUES ('x', 't', 'alice', 1, 1)",
                [],
            )
            .db()?;
        Err(corvus_domain::CorvusError::Conflict("abort".into()))
    });
    assert!(r.is_err());
    assert!(topics(&t).is_empty());
}
