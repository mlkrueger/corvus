//! The in-house migrator ([system/02](../../../../spec/system/02-data-model.md#migrations), D0014).
//!
//! A run is one `BEGIN IMMEDIATE` transaction — that is the lock (P10). Inside it every applied migration's
//! SHA-256 is verified against the embedded file, pending migrations are applied in order and recorded, and
//! `user_version` is set. A non-empty database is copied with `VACUUM INTO` before it is migrated.

use std::path::{Path, PathBuf};

use corvus_domain::CorvusError;
use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::{BUSY_RETRIES, OpenError, sql_err, user_version, vacuum_into};
use crate::clock::Millis;

/// One migration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub version: i64,
    pub name: String,
    pub sql: String,
}

impl Migration {
    pub fn checksum(&self) -> String {
        let digest = Sha256::digest(self.sql.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The migration files compiled into this binary, in order.
const EMBEDDED: &[(i64, &str, &str)] = &[
    (
        1,
        "agents_sessions",
        include_str!("../../migrations/0001_agents_sessions.sql"),
    ),
    (
        2,
        "tasks_claims",
        include_str!("../../migrations/0002_tasks_claims.sql"),
    ),
    (
        3,
        "threads_messages_receipts",
        include_str!("../../migrations/0003_threads_messages_receipts.sql"),
    ),
    (
        4,
        "artifacts_digests_decisions_events",
        include_str!("../../migrations/0004_artifacts_digests_decisions_events.sql"),
    ),
    (
        5,
        "append_only_and_hints",
        include_str!("../../migrations/0005_append_only_and_hints.sql"),
    ),
    (
        6,
        "work_sources",
        include_str!("../../migrations/0006_work_sources.sql"),
    ),
    (
        7,
        "conversations",
        include_str!("../../migrations/0007_conversations.sql"),
    ),
    (
        8,
        "wake_and_reassign",
        include_str!("../../migrations/0008_wake_and_reassign.sql"),
    ),
];

/// The schema version this binary expects (the highest embedded migration).
pub const SCHEMA_VERSION: i64 = 8;

/// The embedded migrations.
pub fn embedded() -> Vec<Migration> {
    EMBEDDED
        .iter()
        .map(|(v, name, sql)| Migration {
            version: *v,
            name: (*name).to_owned(),
            sql: (*sql).to_owned(),
        })
        .collect()
}

/// What a migrator run did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MigrationReport {
    pub from_version: i64,
    pub to_version: i64,
    /// `NNNN_name` of each migration applied by this run.
    pub applied: Vec<String>,
    /// The pre-migration backup written by this run, if any.
    pub backup: Option<PathBuf>,
}

/// A row of `schema_migrations`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AppliedMigration {
    pub version: i64,
    pub name: String,
    pub checksum: String,
    pub applied_at: Millis,
}

/// Lists the applied migrations.
pub fn applied(r: &impl super::Reader) -> corvus_domain::Result<Vec<AppliedMigration>> {
    let mut stmt = r
        .sql()
        .prepare("SELECT version, name, checksum, applied_at FROM schema_migrations ORDER BY version")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AppliedMigration {
                version: row.get(0)?,
                name: row.get(1)?,
                checksum: row.get(2)?,
                applied_at: row.get(3)?,
            })
        })
        .map_err(sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql_err)
}

/// The backup file written before migrating `db_path` to `version`: `corvus.db.pre-v<N>.bak`.
pub fn backup_path(db_path: &Path, version: i64) -> PathBuf {
    let mut name = db_path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".pre-v{version}.bak"));
    db_path.with_file_name(name)
}

pub(super) fn run(
    conn: &mut Connection,
    db_path: &Path,
    migrations: &[Migration],
    now: Millis,
) -> Result<MigrationReport, OpenError> {
    validate_set(migrations)?;
    let latest = migrations.last().map_or(0, |m| m.version);
    let sql = |e| OpenError::Sql(sql_err(e));

    let current = user_version(conn).map_err(sql)?;
    if current > latest {
        return Err(OpenError::Newer {
            database: current,
            binary: latest,
        });
    }

    // VACUUM INTO cannot run inside a transaction, so the backup is taken first. If several processes race
    // here, the first writes the file and the rest find it present; the state is the same either way because
    // migrations are atomic.
    let mut backup = None;
    if current < latest && !is_empty(conn).map_err(sql)? {
        let dest = backup_path(db_path, latest);
        if !dest.exists() {
            match vacuum_into(conn, &dest) {
                Ok(()) => backup = Some(dest),
                Err(CorvusError::Conflict(_)) => {}
                Err(e) => return Err(OpenError::Sql(e)),
            }
        }
    }

    let mut attempt = 0;
    loop {
        match apply(conn, migrations, latest, now) {
            Err(OpenError::Sql(CorvusError::Busy)) if attempt < BUSY_RETRIES => attempt += 1,
            Err(e) => return Err(e),
            Ok(applied) => {
                if !applied.is_empty() {
                    tracing::info!(op = "migrate", from = current, to = latest, applied = %applied.join(","), "applied migrations");
                }
                return Ok(MigrationReport {
                    from_version: current,
                    to_version: latest,
                    applied,
                    backup,
                });
            }
        }
    }
}

fn apply(
    conn: &mut Connection,
    migrations: &[Migration],
    latest: i64,
    now: Millis,
) -> Result<Vec<String>, OpenError> {
    let sql = |e| OpenError::Sql(sql_err(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           version    INTEGER PRIMARY KEY,
           name       TEXT NOT NULL,
           checksum   TEXT NOT NULL,
           applied_at INTEGER NOT NULL
         ) STRICT;",
    )
    .map_err(sql)?;

    let current = user_version(&tx).map_err(sql)?;
    if current > latest {
        return Err(OpenError::Newer {
            database: current,
            binary: latest,
        });
    }

    let done: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
            .map_err(sql)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql)?
    };
    for (version, checksum) in &done {
        let Some(m) = migrations.iter().find(|m| m.version == *version) else {
            return Err(if *version > latest {
                OpenError::Newer {
                    database: *version,
                    binary: latest,
                }
            } else {
                OpenError::UnknownMigration { version: *version }
            });
        };
        if m.checksum() != *checksum {
            return Err(OpenError::Checksum {
                version: m.version,
                name: m.name.clone(),
            });
        }
    }

    let mut applied = Vec::new();
    for m in migrations
        .iter()
        .filter(|m| !done.iter().any(|(v, _)| *v == m.version))
    {
        tx.execute_batch(&m.sql).map_err(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES (?1, ?2, ?3, ?4)",
            params![m.version, m.name, m.checksum(), now],
        )
        .map_err(sql)?;
        applied.push(format!("{:04}_{}", m.version, m.name));
    }
    if current != latest {
        tx.execute_batch(&format!("PRAGMA user_version = {latest}"))
            .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(applied)
}

fn is_empty(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT count(*) = 0 FROM sqlite_schema", [], |r| r.get(0))
}

fn validate_set(migrations: &[Migration]) -> Result<(), OpenError> {
    for (i, m) in migrations.iter().enumerate() {
        if m.version != i as i64 + 1 {
            return Err(OpenError::Sql(CorvusError::Internal(format!(
                "migrations must be numbered 1..N without gaps; found {} at position {}",
                m.version,
                i + 1
            ))));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_set_is_contiguous_and_matches_schema_version() {
        let m = embedded();
        validate_set(&m).unwrap();
        assert_eq!(m.last().unwrap().version, SCHEMA_VERSION);
        assert!(m.iter().all(|m| !m.sql.trim().is_empty()));
    }

    #[test]
    fn checksum_is_sha256_hex() {
        let m = Migration {
            version: 1,
            name: "x".into(),
            sql: "abc".into(),
        };
        assert_eq!(
            m.checksum(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn backup_path_shape() {
        assert_eq!(
            backup_path(Path::new("/s/corvus.db"), 6),
            PathBuf::from("/s/corvus.db.pre-v6.bak")
        );
    }
}
