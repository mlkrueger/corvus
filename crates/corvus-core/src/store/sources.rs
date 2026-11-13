//! Work-source tables (`source_settings`, `source_tickets`, `source_links`, `source_sync_state`), system/07.

use std::collections::BTreeSet;

use corvus_domain::Result;
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

pub fn get_filter(r: &impl Reader, source: &str) -> Result<Option<String>> {
    r.sql()
        .query_row(
            "SELECT filter FROM source_settings WHERE source = ?1",
            [source],
            |r| r.get(0),
        )
        .optional()
        .db()
}

pub fn put_filter(tx: &WriteTx<'_>, source: &str, filter_json: &str, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO source_settings (source, filter, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (source) DO UPDATE SET filter = excluded.filter, updated_at = excluded.updated_at",
            params![source, filter_json, now],
        )
        .db()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct TicketRow {
    pub key: String,
    pub project: String,
    pub summary: String,
    pub status: String,
    pub status_category: String,
    pub priority: i64,
    pub url: String,
    pub updated_stamp: String,
    pub resolved_at: Option<i64>,
    pub assignee: Option<String>,
}

/// Replaces this source's snapshot wholesale.
pub fn replace_tickets(tx: &WriteTx<'_>, source: &str, tickets: &[TicketRow]) -> Result<()> {
    let c = tx.sql_mut();
    c.execute("DELETE FROM source_tickets WHERE source = ?1", [source])
        .db()?;
    let mut st = c
        .prepare(
            "INSERT INTO source_tickets (source, key, project, summary, status, status_category, priority, url,
                                         updated_stamp, resolved_at, assignee)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .db()?;
    for t in tickets {
        st.execute(params![
            source,
            t.key,
            t.project,
            t.summary,
            t.status,
            t.status_category,
            t.priority,
            t.url,
            t.updated_stamp,
            t.resolved_at,
            t.assignee
        ])
        .db()?;
    }
    Ok(())
}

pub fn tickets(r: &impl Reader, source: &str) -> Result<Vec<TicketRow>> {
    let mut st = r
        .sql()
        .prepare(
            "SELECT key, coalesce(project, ''), coalesce(summary, ''), coalesce(status, ''),
                    coalesce(status_category, 'todo'), coalesce(priority, 0), coalesce(url, ''),
                    coalesce(updated_stamp, ''), resolved_at, assignee
               FROM source_tickets WHERE source = ?1 ORDER BY status, priority DESC, key",
        )
        .db()?;
    let rows = st
        .query_map([source], |r| {
            Ok(TicketRow {
                key: r.get(0)?,
                project: r.get(1)?,
                summary: r.get(2)?,
                status: r.get(3)?,
                status_category: r.get(4)?,
                priority: r.get(5)?,
                url: r.get(6)?,
                updated_stamp: r.get(7)?,
                resolved_at: r.get(8)?,
                assignee: r.get(9)?,
            })
        })
        .db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRow {
    pub id: i64,
    pub ticket_key: String,
    pub task_id: String,
    pub updated_stamp: String,
    pub comment_hwm: i64,
    pub hwm_ids: BTreeSet<String>,
    pub in_scope: bool,
    pub manual: bool,
    /// Joined from `tasks`.
    pub task_status: String,
}

const LINK_SELECT: &str = "SELECT l.id, l.ticket_key, l.task_id, l.updated_stamp, l.comment_hwm, l.hwm_ids,
        l.in_scope, l.manual, t.status
   FROM source_links l JOIN tasks t ON t.id = l.task_id";

fn link_row(r: &Row<'_>) -> rusqlite::Result<LinkRow> {
    let ids: String = r.get(5)?;
    Ok(LinkRow {
        id: r.get(0)?,
        ticket_key: r.get(1)?,
        task_id: r.get(2)?,
        updated_stamp: r.get(3)?,
        comment_hwm: r.get(4)?,
        hwm_ids: serde_json::from_str(&ids).unwrap_or_default(),
        in_scope: r.get(6)?,
        manual: r.get(7)?,
        task_status: r.get(8)?,
    })
}

/// The newest link for a ticket: the live link.
pub fn live_link(r: &impl Reader, source: &str, key: &str) -> Result<Option<LinkRow>> {
    r.sql()
        .query_row(
            &format!("{LINK_SELECT} WHERE l.source = ?1 AND l.ticket_key = ?2 ORDER BY l.id DESC LIMIT 1"),
            [source, key],
            link_row,
        )
        .optional()
        .db()
}

/// The live link of every ticket of this source.
pub fn live_links(r: &impl Reader, source: &str) -> Result<Vec<LinkRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{LINK_SELECT} WHERE l.source = ?1
               AND l.id = (SELECT max(id) FROM source_links x WHERE x.source = l.source AND x.ticket_key = l.ticket_key)"
        ))
        .db()?;
    let rows = st.query_map([source], link_row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// How many links (live or not) this ticket has had.
pub fn link_count(r: &impl Reader, source: &str, key: &str) -> Result<i64> {
    r.sql()
        .query_row(
            "SELECT count(*) FROM source_links WHERE source = ?1 AND ticket_key = ?2",
            [source, key],
            |r| r.get(0),
        )
        .db()
}

#[allow(clippy::too_many_arguments)]
pub fn insert_link(
    tx: &WriteTx<'_>,
    source: &str,
    key: &str,
    task_id: &str,
    updated_stamp: &str,
    hwm: i64,
    hwm_ids: &BTreeSet<String>,
    manual: bool,
    now: i64,
) -> Result<()> {
    let ids = serde_json::to_string(hwm_ids).unwrap_or_else(|_| "[]".into());
    tx.sql_mut()
        .execute(
            "INSERT INTO source_links (source, ticket_key, task_id, updated_stamp, comment_hwm, hwm_ids, in_scope,
                                       manual, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)",
            params![source, key, task_id, updated_stamp, hwm, ids, manual, now],
        )
        .db()?;
    Ok(())
}

pub fn update_link_mark(
    tx: &WriteTx<'_>,
    id: i64,
    updated_stamp: &str,
    hwm: i64,
    hwm_ids: &BTreeSet<String>,
) -> Result<()> {
    let ids = serde_json::to_string(hwm_ids).unwrap_or_else(|_| "[]".into());
    tx.sql_mut()
        .execute(
            "UPDATE source_links SET updated_stamp = ?2, comment_hwm = ?3, hwm_ids = ?4 WHERE id = ?1",
            params![id, updated_stamp, hwm, ids],
        )
        .db()?;
    Ok(())
}

pub fn set_in_scope(tx: &WriteTx<'_>, id: i64, in_scope: bool) -> Result<()> {
    tx.sql_mut()
        .execute(
            "UPDATE source_links SET in_scope = ?2 WHERE id = ?1",
            params![id, in_scope],
        )
        .db()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyncStateRow {
    pub last_sync_at: Option<i64>,
    pub last_stats: Option<String>,
    pub last_error: Option<String>,
}

pub fn sync_state(r: &impl Reader, source: &str) -> Result<SyncStateRow> {
    Ok(r.sql()
        .query_row(
            "SELECT last_sync_at, last_stats, last_error FROM source_sync_state WHERE source = ?1",
            [source],
            |r| {
                Ok(SyncStateRow {
                    last_sync_at: r.get(0)?,
                    last_stats: r.get(1)?,
                    last_error: r.get(2)?,
                })
            },
        )
        .optional()
        .db()?
        .unwrap_or_default())
}

pub fn record_success(tx: &WriteTx<'_>, source: &str, now: i64, stats_json: &str) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO source_sync_state (source, last_sync_at, last_stats, last_error) VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT (source) DO UPDATE SET last_sync_at = excluded.last_sync_at,
                                                last_stats = excluded.last_stats, last_error = NULL",
            params![source, now, stats_json],
        )
        .db()?;
    Ok(())
}

pub fn record_error(tx: &WriteTx<'_>, source: &str, error: &str) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO source_sync_state (source, last_error) VALUES (?1, ?2)
             ON CONFLICT (source) DO UPDATE SET last_error = excluded.last_error",
            params![source, error],
        )
        .db()?;
    Ok(())
}
