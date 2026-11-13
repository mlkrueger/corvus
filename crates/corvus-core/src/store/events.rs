use corvus_domain::{EventKind, Result};
use rusqlite::params;

use crate::db::{SqlResultExt, WriteTx};

/// An audit row. `summary` never contains a message or task body.
#[derive(Debug, Default, Clone)]
pub struct NewEvent<'a> {
    pub summary: String,
    pub agent_name: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub message_id: Option<i64>,
}

pub fn insert(tx: &WriteTx<'_>, kind: EventKind, e: &NewEvent<'_>, now: i64) -> Result<i64> {
    tx.sql_mut()
        .execute(
            "INSERT INTO events (kind, summary, agent_name, session_id, task_id, thread_id, message_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![kind.as_str(), e.summary, e.agent_name, e.session_id, e.task_id, e.thread_id, e.message_id, now],
        )
        .db()?;
    Ok(tx.sql_mut().last_insert_rowid())
}

/// Shortens free text (titles) for an event summary.
pub fn clip(s: &str) -> String {
    const MAX: usize = 80;
    if s.chars().count() <= MAX {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(MAX - 1).collect();
        out.push('…');
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub kind: String,
    pub summary: String,
    pub agent_name: Option<String>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<i64>,
    pub created_at: i64,
}

/// Newest first, `id < before_id`.
pub fn list(r: &impl crate::db::Reader, before_id: Option<i64>, limit: u32) -> Result<Vec<EventRow>> {
    let mut st = r
        .sql()
        .prepare(
            "SELECT id, kind, summary, agent_name, session_id, task_id, thread_id, message_id, created_at
               FROM events WHERE (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2",
        )
        .db()?;
    let rows = st
        .query_map(params![before_id, limit], |r| {
            Ok(EventRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                summary: r.get(2)?,
                agent_name: r.get(3)?,
                session_id: r.get(4)?,
                task_id: r.get(5)?,
                thread_id: r.get(6)?,
                message_id: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
