use corvus_domain::{ConversationRef, Result};
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRow {
    pub tool: String,
    pub conversation_id: String,
    pub agent_name: String,
    pub directory: Option<String>,
    pub carried_from: Option<String>,
    pub created_at: i64,
}

fn row(r: &Row<'_>) -> rusqlite::Result<ConversationRow> {
    Ok(ConversationRow {
        tool: r.get(0)?,
        conversation_id: r.get(1)?,
        agent_name: r.get(2)?,
        directory: r.get(3)?,
        carried_from: r.get(4)?,
        created_at: r.get(5)?,
    })
}

/// Links a conversation to an agent. Returns `false` if the conversation is already linked (unchanged).
pub fn insert(
    tx: &WriteTx<'_>,
    conversation: &ConversationRef,
    agent_name: &str,
    directory: Option<&str>,
    carried_from: Option<&str>,
    now: i64,
) -> Result<bool> {
    let n = tx
        .sql_mut()
        .execute(
            "INSERT INTO conversations (tool, conversation_id, agent_name, directory, carried_from, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (tool, conversation_id) DO NOTHING",
            params![
                conversation.tool().as_str(),
                conversation.id(),
                agent_name,
                directory,
                carried_from,
                now
            ],
        )
        .db()?;
    Ok(n == 1)
}

pub fn get(r: &impl Reader, conversation: &ConversationRef) -> Result<Option<ConversationRow>> {
    r.sql()
        .query_row(
            "SELECT tool, conversation_id, agent_name, directory, carried_from, created_at
               FROM conversations WHERE tool = ?1 AND conversation_id = ?2",
            params![conversation.tool().as_str(), conversation.id()],
            row,
        )
        .optional()
        .db()
}

/// An agent's conversations, newest first.
pub fn for_agent(r: &impl Reader, agent_name: &str) -> Result<Vec<ConversationRow>> {
    let mut stmt = r
        .sql()
        .prepare(
            "SELECT tool, conversation_id, agent_name, directory, carried_from, created_at
               FROM conversations WHERE agent_name = ?1 ORDER BY created_at DESC, rowid DESC",
        )
        .db()?;
    let rows = stmt.query_map(params![agent_name], row).db()?;
    rows.collect::<rusqlite::Result<Vec<_>>>().db()
}

/// Every agent's newest conversation, by agent name.
pub fn newest_per_agent(r: &impl Reader) -> Result<Vec<ConversationRow>> {
    let mut stmt = r
        .sql()
        .prepare(
            "SELECT tool, conversation_id, agent_name, directory, carried_from, created_at
               FROM conversations c
              WHERE c.rowid = (SELECT c2.rowid FROM conversations c2
                                WHERE c2.agent_name = c.agent_name
                                ORDER BY c2.created_at DESC, c2.rowid DESC LIMIT 1)
              ORDER BY agent_name",
        )
        .db()?;
    let rows = stmt.query_map([], row).db()?;
    rows.collect::<rusqlite::Result<Vec<_>>>().db()
}
