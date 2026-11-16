//! Messages (append-only) and the unread / unresolved-question queries.

use corvus_domain::{MessageType, Result};
use rusqlite::{OptionalExtension, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

// Deliberately not `Debug`: it carries a body, which must never reach a log line.
#[allow(missing_debug_implementations)]
pub struct NewMessage<'a> {
    pub thread_id: &'a str,
    pub sender: &'a str,
    pub recipient: Option<&'a str>,
    pub kind: MessageType,
    pub in_reply_to: Option<i64>,
    pub body: &'a str,
    pub session_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
    /// A broadcast that wakes idle sessions.
    pub wake: bool,
    /// The original message, when this is a reassigned copy.
    pub reassigned_from: Option<i64>,
}

/// Inserts a message (its hint is written by trigger). Returns the new id.
pub fn insert(tx: &WriteTx<'_>, m: &NewMessage<'_>, now: i64) -> Result<i64> {
    tx.sql_mut()
        .execute(
            "INSERT INTO messages (thread_id, sender, recipient, type, in_reply_to, body, session_id, idempotency_key,
                                   created_at, wake, reassigned_from)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.thread_id,
                m.sender,
                m.recipient,
                m.kind.as_str(),
                m.in_reply_to,
                m.body,
                m.session_id,
                m.idempotency_key,
                now,
                m.wake,
                m.reassigned_from
            ],
        )
        .db()?;
    Ok(tx.sql_mut().last_insert_rowid())
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: i64,
    pub thread_id: String,
    pub sender: String,
    pub recipient: Option<String>,
    pub kind: String,
    pub in_reply_to: Option<i64>,
    pub body: String,
    pub session_id: Option<String>,
    pub created_at: i64,
    pub wake: bool,
    pub reassigned_from: Option<i64>,
}

const SELECT: &str =
    "SELECT m.id, m.thread_id, m.sender, m.recipient, m.type, m.in_reply_to, m.body, m.session_id,
        m.created_at, m.wake, m.reassigned_from FROM messages m";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        thread_id: r.get(1)?,
        sender: r.get(2)?,
        recipient: r.get(3)?,
        kind: r.get(4)?,
        in_reply_to: r.get(5)?,
        body: r.get(6)?,
        session_id: r.get(7)?,
        created_at: r.get(8)?,
        wake: r.get(9)?,
        reassigned_from: r.get(10)?,
    })
}

/// The one definition of "unread for agent A" (system/02 receipts).
const UNREAD_FOR: &str = "(m.recipient = ?1 OR m.recipient IS NULL) AND m.sender <> ?1
        AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id AND r.agent_name = ?1)";

fn collect(st: &mut rusqlite::Statement<'_>, p: impl rusqlite::Params) -> Result<Vec<MessageRow>> {
    let rows = st.query_map(p, row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

pub fn get(r: &impl Reader, id: i64) -> Result<Option<MessageRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE m.id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn by_idempotency_key(r: &impl Reader, sender: &str, key: &str) -> Result<Option<MessageRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE m.sender = ?1 AND m.idempotency_key = ?2"),
            [sender, key],
            row,
        )
        .optional()
        .db()
}

/// Unread for `agent`, oldest first, ids above `after_id`.
pub fn unread(r: &impl Reader, agent: &str, after_id: i64, limit: u32) -> Result<Vec<MessageRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE {UNREAD_FOR} AND m.id > ?2 ORDER BY m.id LIMIT ?3"
        ))
        .db()?;
    collect(&mut st, params![agent, after_id, limit])
}

/// Unread mail addressed to `agent` (no broadcasts), oldest first: what a reassignment moves (D0025).
pub fn unread_direct(r: &impl Reader, agent: &str) -> Result<Vec<MessageRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE m.recipient = ?1 AND m.sender <> ?1
               AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id AND r.agent_name = ?1)
             ORDER BY m.id"
        ))
        .db()?;
    collect(&mut st, params![agent])
}

/// Unread direct-mail counts, for every agent that has any.
pub fn unread_direct_counts(r: &impl Reader) -> Result<Vec<(String, i64)>> {
    let mut st = r
        .sql()
        .prepare(
            "SELECT m.recipient, count(*) FROM messages m
              WHERE m.recipient IS NOT NULL AND m.sender <> m.recipient
                AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id
                                 AND r.agent_name = m.recipient)
              GROUP BY m.recipient ORDER BY m.recipient",
        )
        .db()?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Unread counts per agent, for every agent that has had a session.
pub fn unread_counts(r: &impl Reader) -> Result<Vec<(String, i64)>> {
    let mut st = r
        .sql()
        .prepare(
            "SELECT a.agent_name,
                    (SELECT count(*) FROM messages m
                      WHERE (m.recipient = a.agent_name OR m.recipient IS NULL) AND m.sender <> a.agent_name
                        AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id
                                         AND r.agent_name = a.agent_name))
               FROM (SELECT DISTINCT agent_name FROM sessions) a
              ORDER BY a.agent_name",
        )
        .db()?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Questions addressed to `agent` (or broadcast, not sent by it) that no `answer` replies to.
pub fn unresolved_questions_for(r: &impl Reader, agent: &str, limit: u32) -> Result<Vec<MessageRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE m.type = 'question' AND (m.recipient = ?1 OR m.recipient IS NULL) AND m.sender <> ?1
               AND NOT EXISTS (SELECT 1 FROM messages a WHERE a.type = 'answer' AND a.in_reply_to = m.id)
             ORDER BY m.id LIMIT ?2"
        ))
        .db()?;
    collect(&mut st, params![agent, limit])
}

pub fn in_thread(r: &impl Reader, thread: &str, after_id: i64, limit: u32) -> Result<Vec<MessageRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE m.thread_id = ?1 AND m.id > ?2 ORDER BY m.id LIMIT ?3"
        ))
        .db()?;
    collect(&mut st, params![thread, after_id, limit])
}

/// Every agent that has ever had a session: the audience of a broadcast (minus its sender).
pub fn broadcast_audience(r: &impl Reader) -> Result<Vec<String>> {
    let mut st = r
        .sql()
        .prepare("SELECT DISTINCT agent_name FROM sessions ORDER BY agent_name")
        .db()?;
    let rows = st.query_map([], |r| r.get(0)).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// The operator listing: newest first, `id < before_id`, optional filters. `unread_only` keeps messages that
/// some member of their audience has not acked.
pub fn list(
    r: &impl Reader,
    before_id: Option<i64>,
    agent: Option<&str>,
    thread: Option<&str>,
    unread_only: bool,
    limit: u32,
) -> Result<Vec<MessageRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT}
              WHERE (?1 IS NULL OR m.id < ?1)
                AND (?2 IS NULL OR m.sender = ?2 OR m.recipient = ?2)
                AND (?3 IS NULL OR m.thread_id = ?3)
                AND (NOT ?4 OR CASE WHEN m.recipient IS NOT NULL
                       THEN NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id AND r.agent_name = m.recipient)
                       ELSE EXISTS (SELECT 1 FROM (SELECT DISTINCT agent_name FROM sessions) a
                                     WHERE a.agent_name <> m.sender
                                       AND NOT EXISTS (SELECT 1 FROM receipts r WHERE r.message_id = m.id
                                                        AND r.agent_name = a.agent_name))
                     END)
              ORDER BY m.id DESC
              LIMIT ?5"
        ))
        .db()?;
    collect(&mut st, params![before_id, agent, thread, unread_only, limit])
}
