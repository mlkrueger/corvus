//! The minimal thread operations Huginn needs (a task's thread). Plan 0003's Muninn owns threads in full.

use corvus_domain::Result;
use rusqlite::{OptionalExtension, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

pub fn insert_for_task(
    tx: &WriteTx<'_>,
    id: &str,
    task_id: &str,
    title: &str,
    creator: &str,
    now: i64,
) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO threads (id, title, task_id, creator, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, title, task_id, creator, now],
        )
        .db()?;
    Ok(())
}

pub fn for_task(r: &impl Reader, task_id: &str) -> Result<Option<String>> {
    r.sql()
        .query_row("SELECT id FROM threads WHERE task_id = ?1", [task_id], |r| {
            r.get(0)
        })
        .optional()
        .db()
}
