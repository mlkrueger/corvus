use corvus_domain::Result;
use rusqlite::params;

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

/// Agents that have acked `message`, alphabetically.
pub fn readers(r: &impl Reader, message: i64) -> Result<Vec<String>> {
    let mut st = r
        .sql()
        .prepare("SELECT agent_name FROM receipts WHERE message_id = ?1 ORDER BY agent_name")
        .db()?;
    let rows = st.query_map([message], |r| r.get(0)).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
