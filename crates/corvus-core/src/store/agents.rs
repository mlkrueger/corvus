use corvus_domain::{AgentName, Result};
use rusqlite::{OptionalExtension, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

/// Creates the agent if it does not exist. Returns `true` if this call created it — for a generated name that
/// means the name is now reserved for the caller.
pub fn ensure(tx: &WriteTx<'_>, name: &AgentName, now: i64) -> Result<bool> {
    let created: Option<String> = tx
        .sql_mut()
        .query_row(
            "INSERT INTO agents (name, created_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING RETURNING name",
            params![name.as_str(), now],
            |r| r.get(0),
        )
        .optional()
        .db()?;
    Ok(created.is_some())
}

pub fn exists(r: &impl Reader, name: &AgentName) -> Result<bool> {
    r.sql()
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM agents WHERE name = ?1)",
            [name.as_str()],
            |r| r.get(0),
        )
        .db()
}
