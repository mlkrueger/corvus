use corvus_domain::Result;
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    pub id: String,
    pub title: String,
    pub decision: String,
    pub rationale: String,
    pub decided_by: String,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub supersedes: Option<String>,
    pub created_at: i64,
    /// Derived: the newest decision that supersedes this one.
    pub superseded_by: Option<String>,
}

const SELECT: &str = "SELECT d.id, d.title, d.decision, d.rationale, d.decided_by, d.task_id, d.thread_id,
        d.supersedes, d.created_at,
        (SELECT s.id FROM decisions s WHERE s.supersedes = d.id ORDER BY s.created_at DESC, s.rowid DESC LIMIT 1)
   FROM decisions d";

fn row(r: &Row<'_>) -> rusqlite::Result<DecisionRow> {
    Ok(DecisionRow {
        id: r.get(0)?,
        title: r.get(1)?,
        decision: r.get(2)?,
        rationale: r.get(3)?,
        decided_by: r.get(4)?,
        task_id: r.get(5)?,
        thread_id: r.get(6)?,
        supersedes: r.get(7)?,
        created_at: r.get(8)?,
        superseded_by: r.get(9)?,
    })
}

#[derive(Debug)]
pub struct NewDecision<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub decision: &'a str,
    pub rationale: &'a str,
    pub decided_by: &'a str,
    pub task_id: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub supersedes: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
}

/// Inserts unless `(decided_by, idempotency_key)` exists. Returns `false` on a replay.
pub fn insert(tx: &WriteTx<'_>, d: &NewDecision<'_>, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO decisions (id, title, decision, rationale, decided_by, task_id, thread_id, supersedes,
                                    idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (decided_by, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
            params![
                d.id,
                d.title,
                d.decision,
                d.rationale,
                d.decided_by,
                d.task_id,
                d.thread_id,
                d.supersedes,
                d.idempotency_key,
                now
            ],
        )
        .db()?;
    Ok(n == 1)
}

pub fn get(r: &impl Reader, id: &str) -> Result<Option<DecisionRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE d.id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn by_idempotency_key(r: &impl Reader, by: &str, key: &str) -> Result<Option<DecisionRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE d.decided_by = ?1 AND d.idempotency_key = ?2"),
            [by, key],
            row,
        )
        .optional()
        .db()
}

/// Newest first, optionally filtered; superseded decisions only when asked.
pub fn list(
    r: &impl Reader,
    task: Option<&str>,
    thread: Option<&str>,
    include_superseded: bool,
    limit: u32,
) -> Result<Vec<DecisionRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT}
              WHERE (?1 IS NULL OR d.task_id = ?1) AND (?2 IS NULL OR d.thread_id = ?2)
                AND (?3 OR NOT EXISTS (SELECT 1 FROM decisions s WHERE s.supersedes = d.id))
              ORDER BY d.created_at DESC, d.rowid DESC
              LIMIT ?4"
        ))
        .db()?;
    let rows = st
        .query_map(params![task, thread, include_superseded, limit], row)
        .db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
