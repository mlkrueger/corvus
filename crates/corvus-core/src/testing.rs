//! Test harness: a fresh database file in a temp directory per test, a manual clock, and fixtures.
//! Available to this crate's tests and, with the `test-support` feature, to other crates' tests.
//!
//! Fixtures insert rows directly (services arrive in plan 0002) and go through [`Database::write`], so they
//! obey the same transaction rules as production code.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use corvus_domain::{AgentName, Result, SessionId, TaskId};
use rusqlite::{OptionalExtension, params};

use crate::clock::{Clock, ManualClock};
use crate::db::{Database, MigrationReport, SqlResultExt, WriteTx};
use crate::ids::{new_session_id, new_task_id};
use crate::services::{Services, Settings};

/// A migrated database in its own temp directory. The directory is removed on drop.
#[derive(Debug)]
pub struct TestDb {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub clock: Arc<ManualClock>,
    pub db: Database,
    pub report: MigrationReport,
}

impl TestDb {
    /// A fresh, fully migrated database file.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("state").join("corvus.db");
        let clock = Arc::new(ManualClock::default());
        let (db, report) = Database::open(&path, clock.as_ref()).expect("open test database");
        Self {
            dir,
            path,
            clock,
            db,
            report,
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Opens a second, independent handle (its own connections) on the same file.
    pub fn reopen(&self) -> Database {
        Database::open(&self.path, self.clock.as_ref()).expect("reopen").0
    }

    /// Services on a fresh handle to this database, sharing the test's manual clock.
    pub fn services(&self) -> Services {
        self.services_with(Settings::default())
    }

    pub fn services_with(&self, settings: Settings) -> Services {
        Services::new(Arc::new(self.reopen()), self.clock.clone(), settings)
    }

    pub fn agent(&self, name: &str) -> AgentName {
        let n = AgentName::parse(name).expect("valid fixture agent name");
        let now = self.now();
        self.db
            .write(|tx| insert_agent(tx, &n, now))
            .expect("insert agent");
        n
    }

    pub fn session(&self, agent: &str) -> SessionId {
        let n = self.agent(agent);
        let now = self.now();
        self.db
            .write(|tx| insert_session(tx, &n, now))
            .expect("insert session")
    }

    pub fn task(&self, creator: &str, title: &str) -> TaskId {
        self.task_with(
            creator,
            TaskFixture {
                title,
                ..TaskFixture::default()
            },
        )
    }

    pub fn task_with(&self, creator: &str, f: TaskFixture<'_>) -> TaskId {
        let c = self.agent(creator);
        let now = self.now();
        self.db
            .write(|tx| insert_task(tx, &c, &f, now))
            .expect("insert task")
    }
}

impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

/// Optional task fields for [`TestDb::task_with`].
#[derive(Debug, Clone)]
pub struct TaskFixture<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub priority: i64,
    pub pool: &'a str,
    pub status: &'a str,
}

impl Default for TaskFixture<'_> {
    fn default() -> Self {
        Self {
            title: "a task",
            body: "",
            priority: 0,
            pool: "default",
            status: "open",
        }
    }
}

pub fn insert_agent(tx: &WriteTx<'_>, name: &AgentName, now: i64) -> Result<()> {
    tx.sql_mut()
        .execute(
            "INSERT INTO agents (name, created_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![name.as_str(), now],
        )
        .db()?;
    Ok(())
}

pub fn insert_session(tx: &WriteTx<'_>, agent: &AgentName, now: i64) -> Result<SessionId> {
    let id = new_session_id();
    tx.sql_mut()
        .execute(
            "INSERT INTO sessions (id, agent_name, client_name, started_at, last_seen_at)
             VALUES (?1, ?2, 'test', ?3, ?3)",
            params![id.to_string(), agent.as_str(), now],
        )
        .db()?;
    Ok(id)
}

pub fn insert_task(tx: &WriteTx<'_>, creator: &AgentName, f: &TaskFixture<'_>, now: i64) -> Result<TaskId> {
    let id = new_task_id();
    tx.sql_mut()
        .execute(
            "INSERT INTO tasks (id, title, body, priority, pool, status, creator, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                id.to_string(),
                f.title,
                f.body,
                f.priority,
                f.pool,
                f.status,
                creator.as_str(),
                now
            ],
        )
        .db()?;
    Ok(id)
}

/// Counts rows in `table` (a trusted, test-supplied identifier).
pub fn count(db: &Database, table: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {table}");
    db.read(|tx| crate::db::Reader::sql(tx).query_row(&sql, [], |r| r.get(0)).db())
        .expect("count")
}

/// Runs one SQL statement in a write transaction and returns rows changed (for constraint tests).
pub fn exec(db: &Database, sql: &str) -> Result<usize> {
    db.write(|tx| tx.sql_mut().execute(sql, []).db())
}

/// Reads a single optional integer.
pub fn query_i64(db: &Database, sql: &str) -> Option<i64> {
    db.read(|tx| {
        crate::db::Reader::sql(tx)
            .query_row(sql, [], |r| r.get(0))
            .optional()
            .db()
    })
    .expect("query")
}

/// File mode bits (`0o777` mask) of `p`.
pub fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).expect("metadata").permissions().mode() & 0o777
}
