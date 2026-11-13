use corvus_domain::{ArtifactRef, Result};
use rusqlite::params;

use crate::db::{Reader, SqlResultExt, WriteTx};

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactRow {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub commit_hash: Option<String>,
    pub description: String,
    pub created_at: i64,
}

/// Which row an artifact is attached to (exactly one).
#[derive(Debug, Clone, Copy)]
pub enum Owner<'a> {
    Task(&'a str),
    Thread(&'a str),
    Message(i64),
}

pub fn insert(
    tx: &WriteTx<'_>,
    id: &str,
    a: &ArtifactRef,
    description: &str,
    owner: Owner<'_>,
    now: i64,
) -> Result<()> {
    let (task, thread, message) = match owner {
        Owner::Task(t) => (Some(t), None, None),
        Owner::Thread(t) => (None, Some(t), None),
        Owner::Message(m) => (None, None, Some(m)),
    };
    tx.sql_mut()
        .execute(
            "INSERT INTO artifacts (id, kind, path, commit_hash, description, task_id, thread_id, message_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, a.kind.as_str(), a.path, a.commit, description, task, thread, message, now],
        )
        .db()?;
    Ok(())
}

pub fn list(r: &impl Reader, owner: Owner<'_>) -> Result<Vec<ArtifactRow>> {
    let (col, key): (&str, rusqlite::types::Value) = match owner {
        Owner::Task(t) => ("task_id", t.to_owned().into()),
        Owner::Thread(t) => ("thread_id", t.to_owned().into()),
        Owner::Message(m) => ("message_id", m.into()),
    };
    let mut st = r
        .sql()
        .prepare(&format!(
            "SELECT id, kind, path, commit_hash, description, created_at FROM artifacts WHERE {col} = ?1
             ORDER BY created_at, rowid"
        ))
        .db()?;
    let rows = st
        .query_map([key], |r| {
            Ok(ArtifactRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                commit_hash: r.get(3)?,
                description: r.get(4)?,
                created_at: r.get(5)?,
            })
        })
        .db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}
