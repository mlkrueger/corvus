//! The minimal message insert Huginn needs (the `status` message sent to a cancelled task's claimant). Plan
//! 0003's Muninn owns messages in full.

use corvus_domain::{MessageType, Result};
use rusqlite::params;

use crate::db::{SqlResultExt, WriteTx};

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
}

/// Inserts a message (its hint is written by trigger). Returns the new id.
pub fn insert(tx: &WriteTx<'_>, m: &NewMessage<'_>, now: i64) -> Result<i64> {
    tx.sql_mut()
        .execute(
            "INSERT INTO messages (thread_id, sender, recipient, type, in_reply_to, body, session_id, idempotency_key,
                                   created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.thread_id,
                m.sender,
                m.recipient,
                m.kind.as_str(),
                m.in_reply_to,
                m.body,
                m.session_id,
                m.idempotency_key,
                now
            ],
        )
        .db()?;
    Ok(tx.sql_mut().last_insert_rowid())
}
