//! Threads.

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

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadRow {
    pub id: String,
    pub title: String,
    pub task_id: Option<String>,
    pub status: String,
    pub creator: String,
    pub key: Option<String>,
    pub created_at: i64,
}

pub fn insert(
    tx: &WriteTx<'_>,
    id: &str,
    title: &str,
    task_id: Option<&str>,
    creator: &str,
    key: Option<&str>,
    now: i64,
) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO threads (id, title, task_id, creator, key, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, title, task_id, creator, key, now],
        )
        .db()?;
    Ok(())
}

pub fn get(r: &impl Reader, id: &str) -> Result<Option<ThreadRow>> {
    r.sql()
        .query_row(
            "SELECT id, title, task_id, status, creator, key, created_at FROM threads WHERE id = ?1",
            [id],
            |r| {
                Ok(ThreadRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    task_id: r.get(2)?,
                    status: r.get(3)?,
                    creator: r.get(4)?,
                    key: r.get(5)?,
                    created_at: r.get(6)?,
                })
            },
        )
        .optional()
        .db()
}

pub fn by_key(r: &impl Reader, key: &str) -> Result<Option<String>> {
    r.sql()
        .query_row("SELECT id FROM threads WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .db()
}
