use corvus_domain::{EndReason, Result};
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimRow {
    pub id: String,
    pub task_id: String,
    pub session_id: String,
    pub agent_name: String,
    pub claimed_at: i64,
    pub lease_expires_at: i64,
    pub released_at: Option<i64>,
    pub end_reason: Option<String>,
}

const SELECT: &str = "SELECT id, task_id, session_id, agent_name, claimed_at, lease_expires_at, released_at,
        end_reason FROM claims";

fn row(r: &Row<'_>) -> rusqlite::Result<ClaimRow> {
    Ok(ClaimRow {
        id: r.get(0)?,
        task_id: r.get(1)?,
        session_id: r.get(2)?,
        agent_name: r.get(3)?,
        claimed_at: r.get(4)?,
        lease_expires_at: r.get(5)?,
        released_at: r.get(6)?,
        end_reason: r.get(7)?,
    })
}

pub fn get(r: &impl Reader, id: &str) -> Result<Option<ClaimRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn active_for_session(r: &impl Reader, session_id: &str) -> Result<Option<ClaimRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE session_id = ?1 AND released_at IS NULL"),
            [session_id],
            row,
        )
        .optional()
        .db()
}

pub fn active_for_task(r: &impl Reader, task_id: &str) -> Result<Option<ClaimRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE task_id = ?1 AND released_at IS NULL"),
            [task_id],
            row,
        )
        .optional()
        .db()
}

/// The most recent claim of `task_id` by `session_id`, active or not.
pub fn latest_for(r: &impl Reader, task_id: &str, session_id: &str) -> Result<Option<ClaimRow>> {
    r.sql()
        .query_row(
            &format!(
                "{SELECT} WHERE task_id = ?1 AND session_id = ?2 ORDER BY claimed_at DESC, rowid DESC LIMIT 1"
            ),
            [task_id, session_id],
            row,
        )
        .optional()
        .db()
}

pub fn insert(tx: &WriteTx<'_>, c: &ClaimRow) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO claims (id, task_id, session_id, agent_name, claimed_at, lease_expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                c.id,
                c.task_id,
                c.session_id,
                c.agent_name,
                c.claimed_at,
                c.lease_expires_at
            ],
        )
        .db()?;
    Ok(())
}

pub fn set_lease(tx: &WriteTx<'_>, id: &str, lease_expires_at: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE claims SET lease_expires_at = ?2 WHERE id = ?1 AND released_at IS NULL",
            params![id, lease_expires_at],
        )
        .db()?;
    Ok(())
}

/// Ends an active claim. Returns `false` if it was not active.
pub fn end(tx: &WriteTx<'_>, id: &str, reason: EndReason, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "UPDATE claims SET released_at = ?2, end_reason = ?3 WHERE id = ?1 AND released_at IS NULL",
            params![id, now, reason.as_str()],
        )
        .db()?;
    Ok(n == 1)
}

/// Active claims whose lease has expired at `now` (`lease_expires_at <= now`).
pub fn expired(tx: &WriteTx<'_>, now: i64) -> Result<Vec<ClaimRow>> {
    let mut st = tx
        .sql_mut()
        .prepare(&format!(
            "{SELECT} WHERE released_at IS NULL AND lease_expires_at <= ?1 ORDER BY lease_expires_at, id"
        ))
        .db()?;
    let rows = st.query_map([now], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// (active leases, active-but-expired leases) at `now`.
pub fn lease_counts(r: &impl Reader, now: i64) -> Result<(i64, i64)> {
    r.sql()
        .query_row(
            "SELECT count(*) FILTER (WHERE lease_expires_at > ?1), count(*) FILTER (WHERE lease_expires_at <= ?1)
               FROM claims WHERE released_at IS NULL",
            [now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .db()
}
