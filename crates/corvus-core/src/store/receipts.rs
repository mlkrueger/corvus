use corvus_domain::Result;
use rusqlite::{OptionalExtension, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

/// Records that `agent` read `message`. Returns `true` if newly acked.
pub fn insert(tx: &WriteTx<'_>, message: i64, agent: &str, session: Option<&str>, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO receipts (message_id, agent_name, session_id, acked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT DO NOTHING",
            params![message, agent, session, now],
        )
        .db()?;
    Ok(n == 1)
}

/// Records that the operator moved `message` from `agent` to `to` (D0025): a receipt that is not an
/// acknowledgement. Returns `true` if newly written.
pub fn insert_reassigned(tx: &WriteTx<'_>, message: i64, agent: &str, to: &str, now: i64) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO receipts (message_id, agent_name, acked_at, kind, reassigned_to)
             VALUES (?1, ?2, ?3, 'reassigned', ?4)
             ON CONFLICT DO NOTHING",
            params![message, agent, now, to],
        )
        .db()?;
    Ok(n == 1)
}

/// Agents that have acked `message`, alphabetically. A reassignment is not an acknowledgement.
pub fn readers(r: &impl Reader, message: i64) -> Result<Vec<String>> {
    let mut st = r
        .sql()
        .prepare("SELECT agent_name FROM receipts WHERE message_id = ?1 AND kind = 'ack' ORDER BY agent_name")
        .db()?;
    let rows = st.query_map([message], |r| r.get(0)).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Who `message` was reassigned to, if the operator moved it.
pub fn reassigned_to(r: &impl Reader, message: i64) -> Result<Option<String>> {
    r.sql()
        .query_row(
            "SELECT reassigned_to FROM receipts WHERE message_id = ?1 AND kind = 'reassigned' LIMIT 1",
            [message],
            |r| r.get(0),
        )
        .optional()
        .db()
}
