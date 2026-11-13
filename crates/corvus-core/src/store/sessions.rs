use corvus_domain::{Result, SessionEndReason};
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: String,
    pub agent_name: String,
    pub client_name: String,
    pub metadata: String,
    pub name_generated: bool,
    pub started_at: i64,
    pub last_seen_at: i64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<String>,
    /// From a join on the active claim.
    pub current_task: Option<String>,
    pub lease_expires_at: Option<i64>,
}

const SELECT: &str = "SELECT s.id, s.agent_name, s.client_name, s.metadata, s.name_generated, s.started_at,
        s.last_seen_at, s.ended_at, s.end_reason, c.task_id, c.lease_expires_at
   FROM sessions s LEFT JOIN claims c ON c.session_id = s.id AND c.released_at IS NULL";

fn row(r: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        agent_name: r.get(1)?,
        client_name: r.get(2)?,
        metadata: r.get(3)?,
        name_generated: r.get(4)?,
        started_at: r.get(5)?,
        last_seen_at: r.get(6)?,
        ended_at: r.get(7)?,
        end_reason: r.get(8)?,
        current_task: r.get(9)?,
        lease_expires_at: r.get(10)?,
    })
}

#[derive(Debug)]
pub struct NewSession<'a> {
    pub id: &'a str,
    pub agent_name: &'a str,
    pub client_name: &'a str,
    pub metadata: &'a str,
    pub name_generated: bool,
    pub idempotency_key: Option<&'a str>,
}

/// Inserts unless `(agent_name, idempotency_key)` already exists. Returns `false` on an idempotent replay.
pub fn insert(tx: &WriteTx<'_>, s: &NewSession<'_>, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO sessions (id, agent_name, client_name, metadata, name_generated, started_at, last_seen_at,
                                   idempotency_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)
             ON CONFLICT (agent_name, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
            params![s.id, s.agent_name, s.client_name, s.metadata, s.name_generated, now, s.idempotency_key],
        )
        .db()?;
    Ok(n == 1)
}

pub fn get(r: &impl Reader, id: &str) -> Result<Option<SessionRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE s.id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn by_idempotency_key(r: &impl Reader, agent: &str, key: &str) -> Result<Option<SessionRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE s.agent_name = ?1 AND s.idempotency_key = ?2"),
            [agent, key],
            row,
        )
        .optional()
        .db()
}

/// For a replayed registration without a name: the generated-name session created with this key.
pub fn generated_by_idempotency_key(r: &impl Reader, key: &str) -> Result<Option<SessionRow>> {
    r.sql()
        .query_row(
            &format!(
                "{SELECT} WHERE s.name_generated = 1 AND s.idempotency_key = ?1 ORDER BY s.started_at LIMIT 1"
            ),
            [key],
            row,
        )
        .optional()
        .db()
}

/// The heartbeat: `last_seen_at = now` on a live session. Returns the agent name, or `None` if the session is
/// unknown or ended.
pub fn touch(tx: &WriteTx<'_>, id: &str, now: i64) -> Result<Option<String>> {
    tx.sql_mut()
        .query_row(
            "UPDATE sessions SET last_seen_at = ?2 WHERE id = ?1 AND ended_at IS NULL RETURNING agent_name",
            params![id, now],
            |r| r.get(0),
        )
        .optional()
        .db()
}

/// Marks a live session ended. Returns `false` if it was already ended (or unknown).
pub fn end(tx: &WriteTx<'_>, id: &str, reason: SessionEndReason, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "UPDATE sessions SET ended_at = ?2, end_reason = ?3 WHERE id = ?1 AND ended_at IS NULL",
            params![id, now, reason.as_str()],
        )
        .db()?;
    Ok(n == 1)
}

/// Live sessions silent since before `cutoff`, oldest first.
pub fn stale(tx: &WriteTx<'_>, cutoff: i64) -> Result<Vec<(String, String)>> {
    let mut st = tx
        .sql_mut()
        .prepare(
            "SELECT id, agent_name FROM sessions WHERE ended_at IS NULL AND last_seen_at < ?1 ORDER BY last_seen_at",
        )
        .db()?;
    let rows = st.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

pub fn list(r: &impl Reader, include_ended: bool, agent: Option<&str>) -> Result<Vec<SessionRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE (?1 OR s.ended_at IS NULL) AND (?2 IS NULL OR s.agent_name = ?2)
             ORDER BY s.started_at DESC, s.id"
        ))
        .db()?;
    let rows = st.query_map(params![include_ended, agent], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
