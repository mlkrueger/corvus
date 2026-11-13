use corvus_domain::{Result, TaskStatus};
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub priority: i64,
    pub status: String,
    pub pool: String,
    pub recipient_agent: Option<String>,
    pub creator: String,
    pub result_summary: Option<String>,
    pub cancel_reason: Option<String>,
    pub cancelled_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub cancelled_at: Option<i64>,
}

const SELECT: &str =
    "SELECT t.id, t.title, t.body, t.priority, t.status, t.pool, t.recipient_agent, t.creator,
        t.result_summary, t.cancel_reason, t.cancelled_by, t.created_at, t.updated_at, t.completed_at,
        t.cancelled_at FROM tasks t";

fn row(r: &Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        title: r.get(1)?,
        body: r.get(2)?,
        priority: r.get(3)?,
        status: r.get(4)?,
        pool: r.get(5)?,
        recipient_agent: r.get(6)?,
        creator: r.get(7)?,
        result_summary: r.get(8)?,
        cancel_reason: r.get(9)?,
        cancelled_by: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
        completed_at: r.get(13)?,
        cancelled_at: r.get(14)?,
    })
}

// Deliberately not `Debug`: it carries a body, which must never reach a log line.
#[allow(missing_debug_implementations)]
pub struct NewTask<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub priority: i64,
    pub pool: &'a str,
    pub recipient_agent: Option<&'a str>,
    pub creator: &'a str,
    pub idempotency_key: Option<&'a str>,
}

/// Inserts unless `(creator, idempotency_key)` exists. Returns `false` on an idempotent replay.
pub fn insert(tx: &WriteTx<'_>, t: &NewTask<'_>, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO tasks (id, title, body, priority, pool, recipient_agent, creator, idempotency_key,
                                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT (creator, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
            params![
                t.id,
                t.title,
                t.body,
                t.priority,
                t.pool,
                t.recipient_agent,
                t.creator,
                t.idempotency_key,
                now
            ],
        )
        .db()?;
    Ok(n == 1)
}

pub fn get(r: &impl Reader, id: &str) -> Result<Option<TaskRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE t.id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn by_idempotency_key(r: &impl Reader, creator: &str, key: &str) -> Result<Option<TaskRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE t.creator = ?1 AND t.idempotency_key = ?2"),
            [creator, key],
            row,
        )
        .optional()
        .db()
}

pub fn exists(r: &impl Reader, id: &str) -> Result<bool> {
    r.sql()
        .query_row("SELECT EXISTS (SELECT 1 FROM tasks WHERE id = ?1)", [id], |r| {
            r.get(0)
        })
        .db()
}

pub fn add_dependency(tx: &WriteTx<'_>, task_id: &str, depends_on: &str) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![task_id, depends_on],
        )
        .db()?;
    Ok(())
}

pub fn dependencies(r: &impl Reader, task_id: &str) -> Result<Vec<String>> {
    let mut st = r
        .sql()
        .prepare("SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1 ORDER BY rowid")
        .db()?;
    let rows = st.query_map([task_id], |r| r.get(0)).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// `(blocked, blocked_by_cancelled)`: some dependency not done / some dependency cancelled.
pub fn flags(r: &impl Reader, task_id: &str) -> Result<(bool, bool)> {
    r.sql()
        .query_row(
            "SELECT coalesce(max(dt.status <> 'done'), 0), coalesce(max(dt.status = 'cancelled'), 0)
               FROM task_dependencies d JOIN tasks dt ON dt.id = d.depends_on_id
              WHERE d.task_id = ?1",
            [task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .db()
}

/// Every transitive dependent of `task_id` (not including it), in breadth-first discovery order.
pub fn dependents(r: &impl Reader, task_id: &str) -> Result<Vec<String>> {
    let mut st = r
        .sql()
        .prepare(
            "WITH RECURSIVE dep(id, depth) AS (
                 SELECT task_id, 1 FROM task_dependencies WHERE depends_on_id = ?1
                 UNION
                 SELECT d.task_id, dep.depth + 1 FROM task_dependencies d JOIN dep ON d.depends_on_id = dep.id
             )
             SELECT id FROM dep GROUP BY id ORDER BY min(depth), id",
        )
        .db()?;
    let rows = st.query_map([task_id], |r| r.get(0)).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// The best claimable task for `agent` in `pool` (any pool if `None`): open, unrouted or routed to the agent,
/// every dependency done; highest priority, then oldest, then insertion order.
pub fn next_claimable(tx: &WriteTx<'_>, pool: Option<&str>, agent: &str) -> Result<Option<String>> {
    tx.sql_mut()
        .query_row(
            "SELECT t.id FROM tasks t
              WHERE t.status = 'open'
                AND (?1 IS NULL OR t.pool = ?1)
                AND (t.recipient_agent IS NULL OR t.recipient_agent = ?2)
                AND NOT EXISTS (SELECT 1 FROM task_dependencies d JOIN tasks dt ON dt.id = d.depends_on_id
                                 WHERE d.task_id = t.id AND dt.status <> 'done')
              ORDER BY t.priority DESC, t.created_at, t.rowid
              LIMIT 1",
            params![pool, agent],
            |r| r.get(0),
        )
        .optional()
        .db()
}

pub fn set_status(tx: &WriteTx<'_>, id: &str, status: TaskStatus, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), now],
        )
        .db()?;
    Ok(())
}

pub fn complete(tx: &WriteTx<'_>, id: &str, summary: Option<&str>, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE tasks SET status = 'done', result_summary = ?2, completed_at = ?3, updated_at = ?3 WHERE id = ?1",
            params![id, summary, now],
        )
        .db()?;
    Ok(())
}

pub fn cancel(tx: &WriteTx<'_>, id: &str, by: &str, reason: Option<&str>, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE tasks SET status = 'cancelled', cancel_reason = ?2, cancelled_by = ?3, cancelled_at = ?4,
                              updated_at = ?4
              WHERE id = ?1",
            params![id, reason, by, now],
        )
        .db()?;
    Ok(())
}

pub fn set_body(tx: &WriteTx<'_>, id: &str, body: &str, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE tasks SET body = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, body, now],
        )
        .db()?;
    Ok(())
}

/// Newest first. `agent` matches the creator, the recipient, or the current holder.
pub fn list(
    r: &impl Reader,
    status: Option<TaskStatus>,
    agent: Option<&str>,
    limit: u32,
) -> Result<Vec<TaskRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT}
              WHERE (?1 IS NULL OR t.status = ?1)
                AND (?2 IS NULL OR t.creator = ?2 OR t.recipient_agent = ?2
                     OR EXISTS (SELECT 1 FROM claims c WHERE c.task_id = t.id AND c.released_at IS NULL
                                AND c.agent_name = ?2))
              ORDER BY t.created_at DESC, t.rowid DESC
              LIMIT ?3"
        ))
        .db()?;
    let rows = st
        .query_map(params![status.map(|s| s.as_str()), agent, limit], row)
        .db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Tasks routed to `agent` that are open, plus tasks it currently holds; newest first.
pub fn routed_to(r: &impl Reader, agent: &str, limit: u32) -> Result<Vec<TaskRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT}
              WHERE (t.recipient_agent = ?1 AND t.status = 'open')
                 OR EXISTS (SELECT 1 FROM claims c WHERE c.task_id = t.id AND c.released_at IS NULL
                             AND c.agent_name = ?1)
              ORDER BY t.priority DESC, t.created_at, t.rowid
              LIMIT ?2"
        ))
        .db()?;
    let rows = st.query_map(params![agent, limit], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Open, unblocked tasks `agent` could claim (any pool), in claim order.
pub fn claimable_for(r: &impl Reader, agent: &str, limit: u32) -> Result<Vec<TaskRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT}
              WHERE t.status = 'open' AND (t.recipient_agent IS NULL OR t.recipient_agent = ?1)
                AND NOT EXISTS (SELECT 1 FROM task_dependencies d JOIN tasks dt ON dt.id = d.depends_on_id
                                 WHERE d.task_id = t.id AND dt.status <> 'done')
              ORDER BY t.priority DESC, t.created_at, t.rowid
              LIMIT ?2"
        ))
        .db()?;
    let rows = st.query_map(params![agent, limit], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Counts by status.
pub fn status_counts(r: &impl Reader) -> Result<Vec<(String, i64)>> {
    let mut st = r
        .sql()
        .prepare("SELECT status, count(*) FROM tasks GROUP BY status ORDER BY status")
        .db()?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Recent tasks with `status`, most recently updated first.
pub fn recent_with_status(r: &impl Reader, status: TaskStatus, limit: u32) -> Result<Vec<TaskRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE t.status = ?1 ORDER BY t.updated_at DESC, t.rowid DESC LIMIT ?2"
        ))
        .db()?;
    let rows = st.query_map(params![status.as_str(), limit], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Open tasks with at least one cancelled dependency.
pub fn blocked_by_cancelled_count(r: &impl Reader) -> Result<i64> {
    r.sql()
        .query_row(
            "SELECT count(*) FROM tasks t WHERE t.status = 'open' AND EXISTS (
                 SELECT 1 FROM task_dependencies d JOIN tasks dt ON dt.id = d.depends_on_id
                  WHERE d.task_id = t.id AND dt.status = 'cancelled')",
            [],
            |r| r.get(0),
        )
        .db()
}

/// Open tasks (open and claimed) for the export, in claim order.
pub fn live(r: &impl Reader) -> Result<Vec<TaskRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE t.status IN ('open', 'claimed') ORDER BY t.priority DESC, t.created_at, t.rowid"
        ))
        .db()?;
    let rows = st.query_map([], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
