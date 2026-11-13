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
