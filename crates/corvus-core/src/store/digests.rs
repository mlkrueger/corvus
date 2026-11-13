use corvus_domain::Result;
use rusqlite::{OptionalExtension, Row, params};

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct DigestRow {
    pub id: i64,
    pub scope: String,
    pub reference: Option<String>,
    pub scope_key: String,
    pub summary: String,
    pub author: String,
    pub covers_through: Option<i64>,
    pub created_at: i64,
}

const SELECT: &str =
    "SELECT id, scope, ref, scope_key, summary, author, covers_through, created_at FROM digests";

fn row(r: &Row<'_>) -> rusqlite::Result<DigestRow> {
    Ok(DigestRow {
        id: r.get(0)?,
        scope: r.get(1)?,
        reference: r.get(2)?,
        scope_key: r.get(3)?,
        summary: r.get(4)?,
        author: r.get(5)?,
        covers_through: r.get(6)?,
        created_at: r.get(7)?,
    })
}

// Deliberately not `Debug`: it carries a summary body.
#[allow(missing_debug_implementations)]
pub struct NewDigest<'a> {
    pub scope: &'a str,
    pub reference: Option<&'a str>,
    pub summary: &'a str,
    pub author: &'a str,
    pub covers_through: Option<i64>,
    pub idempotency_key: Option<&'a str>,
}

/// Inserts unless `(author, idempotency_key)` exists. Returns the new id, or `None` on a replay.
pub fn insert(tx: &WriteTx<'_>, d: &NewDigest<'_>, now: i64) -> Result<Option<i64>> {
    tx.sql_mut()
        .query_row(
            "INSERT INTO digests (scope, ref, summary, author, covers_through, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (author, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING
             RETURNING id",
            params![
                d.scope,
                d.reference,
                d.summary,
                d.author,
                d.covers_through,
                d.idempotency_key,
                now
            ],
            |r| r.get(0),
        )
        .optional()
        .db()
}

pub fn get(r: &impl Reader, id: i64) -> Result<Option<DigestRow>> {
    r.sql()
        .query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)
        .optional()
        .db()
}

pub fn by_idempotency_key(r: &impl Reader, author: &str, key: &str) -> Result<Option<DigestRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE author = ?1 AND idempotency_key = ?2"),
            [author, key],
            row,
        )
        .optional()
        .db()
}

/// The current digest for a scope key (the highest id).
pub fn latest(r: &impl Reader, scope_key: &str) -> Result<Option<DigestRow>> {
    r.sql()
        .query_row(
            &format!("{SELECT} WHERE scope_key = ?1 ORDER BY id DESC LIMIT 1"),
            [scope_key],
            row,
        )
        .optional()
        .db()
}

/// The current digest of every scope key, ordered by scope key.
pub fn latest_all(r: &impl Reader) -> Result<Vec<DigestRow>> {
    let mut st = r
        .sql()
        .prepare(&format!(
            "{SELECT} WHERE id IN (SELECT max(id) FROM digests GROUP BY scope_key) ORDER BY scope_key"
        ))
        .db()?;
    let rows = st.query_map([], row).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
