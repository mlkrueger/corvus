//! The one database opener ([system/01](../../../../spec/system/01-architecture.md#database-access)).
//!
//! Nothing outside this module opens a SQLite connection or sets a pragma. Every connection gets
//! `journal_mode = WAL`, `foreign_keys = ON`, `busy_timeout`, `synchronous = FULL`, `trusted_schema = OFF`,
//! and each pragma's result is read back and verified.
//!
//! Transactions come in two shapes, and the shapes cannot be mixed:
//!
//! - [`Database::write`] issues `BEGIN IMMEDIATE` on the single write connection, checks the schema version,
//!   and hands the closure a [`WriteTx`]. Busy timeouts are retried twice, then reported as
//!   [`CorvusError::Busy`].
//! - [`Database::read`] runs a deferred transaction on a `query_only` reader and hands the closure a
//!   [`ReadTx`]. There is no way to turn a `ReadTx` into a `WriteTx`, so store functions that write (which
//!   take `&WriteTx`) cannot be called from a read path:
//!
//! ```compile_fail
//! # use corvus_core::db::{Database, WriteTx, ReadTx};
//! fn insert_something(_tx: &WriteTx<'_>) {}
//! fn read_path(db: &Database) {
//!     let _ = db.read(|tx: &ReadTx<'_>| { insert_something(tx); Ok(()) });
//! }
//! ```

pub mod migrator;

use std::fmt;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use corvus_domain::{CorvusError, Result};
use rusqlite::{Connection, ErrorCode, OpenFlags, Transaction, TransactionBehavior};

use crate::clock::Clock;
use crate::config::ensure_private_dir;
pub use migrator::{Migration, MigrationReport, SCHEMA_VERSION};

/// `busy_timeout` for normal processes ([defaults #5](../../../../spec/system/11-defaults.md)).
pub const BUSY_TIMEOUT_MS: u32 = 5000;
/// Retries after an exhausted busy timeout before reporting `Busy`.
pub const BUSY_RETRIES: u32 = 2;
/// Idle reader connections kept per process.
const MAX_IDLE_READERS: usize = 4;

/// Why a database could not be opened or migrated.
#[derive(Debug)]
pub enum OpenError {
    /// The database was migrated by a newer Corvus.
    Newer {
        database: i64,
        binary: i64,
    },
    /// An applied migration differs from the embedded file.
    Checksum {
        version: i64,
        name: String,
    },
    /// The database records a migration this binary does not have.
    UnknownMigration {
        version: i64,
    },
    /// A pragma did not take the required value.
    Pragma {
        pragma: &'static str,
        expected: String,
        actual: String,
    },
    /// The database file does not exist (read-only open).
    Missing(PathBuf),
    /// The read-only opener found a schema version different from the binary's.
    SchemaMismatch {
        database: i64,
        binary: i64,
    },
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Sql(CorvusError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer { database, binary } => write!(
                f,
                "the database is at schema version {database}, newer than this Corvus supports ({binary}); \
                 upgrade Corvus"
            ),
            Self::Checksum { version, name } => write!(
                f,
                "migration {version:04}_{name} was edited after it was applied (checksum mismatch); \
                 applied migrations are immutable, nothing was migrated"
            ),
            Self::UnknownMigration { version } => {
                write!(
                    f,
                    "the database records migration {version} which this Corvus does not know"
                )
            }
            Self::Pragma {
                pragma,
                expected,
                actual,
            } => {
                write!(f, "pragma {pragma} is {actual}, expected {expected}")
            }
            Self::Missing(p) => write!(f, "database {} does not exist", p.display()),
            Self::SchemaMismatch { database, binary } => {
                write!(
                    f,
                    "database schema version {database} does not match this Corvus ({binary})"
                )
            }
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Sql(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<CorvusError> for OpenError {
    fn from(e: CorvusError) -> Self {
        Self::Sql(e)
    }
}

impl From<OpenError> for CorvusError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::Sql(inner) => inner,
            other => CorvusError::Internal(other.to_string()),
        }
    }
}

/// Maps a SQLite error onto the service error type.
pub fn sql_err(e: rusqlite::Error) -> CorvusError {
    match &e {
        rusqlite::Error::SqliteFailure(f, msg) => match f.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => CorvusError::Busy,
            ErrorCode::ConstraintViolation => {
                let msg = msg.clone().unwrap_or_else(|| "constraint violated".to_owned());
                // SQLITE_CONSTRAINT_CHECK = 275, SQLITE_CONSTRAINT_NOTNULL = 1299
                if matches!(f.extended_code, 275 | 1299) {
                    CorvusError::Validation(msg)
                } else {
                    CorvusError::Conflict(msg)
                }
            }
            _ => CorvusError::Internal(e.to_string()),
        },
        rusqlite::Error::QueryReturnedNoRows => CorvusError::NotFound("not found".to_owned()),
        _ => CorvusError::Internal(e.to_string()),
    }
}

/// `.db()` on a rusqlite result: map the error with [`sql_err`].
pub trait SqlResultExt<T> {
    fn db(self) -> Result<T>;
}

impl<T> SqlResultExt<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(sql_err)
    }
}

/// Anything SQL can be read through: a [`ReadTx`] or a [`WriteTx`].
pub trait Reader: sealed::Sealed {
    /// The underlying connection, inside the open transaction.
    fn sql(&self) -> &Connection;
}

mod sealed {
    pub trait Sealed {}
}

/// An open `BEGIN IMMEDIATE` transaction on the write connection.
pub struct WriteTx<'c> {
    tx: Transaction<'c>,
}

impl fmt::Debug for WriteTx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WriteTx")
    }
}

impl WriteTx<'_> {
    /// The connection, for statements that write.
    pub fn sql_mut(&self) -> &Connection {
        &self.tx
    }
}

impl sealed::Sealed for WriteTx<'_> {}
impl Reader for WriteTx<'_> {
    fn sql(&self) -> &Connection {
        &self.tx
    }
}

/// An open deferred transaction on a `query_only` reader: a consistent snapshot that cannot write.
pub struct ReadTx<'c> {
    tx: Transaction<'c>,
}

impl fmt::Debug for ReadTx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReadTx")
    }
}

impl sealed::Sealed for ReadTx<'_> {}
impl Reader for ReadTx<'_> {
    fn sql(&self) -> &Connection {
        &self.tx
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Writer,
    Reader,
    /// Opened read-only at the OS level (the hook).
    ReadOnly,
}

/// Write-transaction timings for this handle, so tests can assert the queue stays short (system/09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteStats {
    pub transactions: u64,
    /// Longest wait to obtain the write lock (process mutex + `BEGIN IMMEDIATE`), microseconds.
    pub max_wait_us: u64,
    /// Longest `BEGIN IMMEDIATE` → `COMMIT`, microseconds.
    pub max_hold_us: u64,
}

#[derive(Default)]
struct WriteCounters {
    transactions: AtomicU64,
    max_wait_us: AtomicU64,
    max_hold_us: AtomicU64,
}

/// A write transaction holding the lock longer than this is logged as a warning.
const SLOW_WRITE: Duration = Duration::from_millis(250);

/// A read/write database handle: one write connection behind a mutex and a small pool of readers.
pub struct Database {
    path: PathBuf,
    expected_version: i64,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
    stats: WriteCounters,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Opens (creating if needed) and migrates the database at `path` with the embedded migrations.
    pub fn open(path: &Path, clock: &dyn Clock) -> std::result::Result<(Self, MigrationReport), OpenError> {
        Self::open_with_migrations(path, &migrator::embedded(), clock)
    }

    /// As [`Database::open`] with an explicit migration set. Exists for migrator tests.
    #[doc(hidden)]
    pub fn open_with_migrations(
        path: &Path,
        migrations: &[Migration],
        clock: &dyn Clock,
    ) -> std::result::Result<(Self, MigrationReport), OpenError> {
        prepare_file(path)?;
        let mut writer = open_conn(path, Role::Writer, BUSY_TIMEOUT_MS)?;
        let report = migrator::run(&mut writer, path, migrations, clock.now_ms())?;
        let db = Self {
            path: path.to_owned(),
            expected_version: report.to_version,
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
            stats: WriteCounters::default(),
        };
        Ok((db, report))
    }

    /// Opens an existing, fully migrated database for reading and writing **without migrating**. Fails if
    /// the file is missing or at another schema version. For the hook's one write
    /// ([system/06](../../../../spec/system/06-delivery-helpers.md#session-events)), which must never migrate.
    pub fn open_existing(path: &Path, busy_timeout_ms: u32) -> std::result::Result<Self, OpenError> {
        if !path.is_file() {
            return Err(OpenError::Missing(path.to_owned()));
        }
        let writer = open_conn(path, Role::Writer, busy_timeout_ms)?;
        let v = user_version(&writer).map_err(|e| OpenError::Sql(sql_err(e)))?;
        if v != SCHEMA_VERSION {
            return Err(OpenError::SchemaMismatch {
                database: v,
                binary: SCHEMA_VERSION,
            });
        }
        Ok(Self {
            path: path.to_owned(),
            expected_version: v,
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
            stats: WriteCounters::default(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `f` in a `BEGIN IMMEDIATE` transaction and commits if it returns `Ok`.
    ///
    /// `f` may run more than once (busy retry), so it must not have side effects outside the transaction —
    /// which the rules forbid anyway (no IO inside a write transaction).
    pub fn write<T>(&self, mut f: impl FnMut(&WriteTx<'_>) -> Result<T>) -> Result<T> {
        let asked = Instant::now();
        let mut conn = lock(&self.writer);
        let mut attempt = 0;
        loop {
            match self.write_once(&mut conn, &mut f, asked) {
                Err(CorvusError::Busy) if attempt < BUSY_RETRIES => {
                    attempt += 1;
                    tracing::warn!(attempt, "write transaction busy; retrying");
                }
                other => return other,
            }
        }
    }

    fn write_once<T>(
        &self,
        conn: &mut Connection,
        f: &mut impl FnMut(&WriteTx<'_>) -> Result<T>,
        asked: Instant,
    ) -> Result<T> {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .db()?;
        let began = Instant::now();
        let _timer = HoldTimer {
            stats: &self.stats,
            began,
            waited: began - asked,
        };
        let version = user_version(&tx).db()?;
        if version != self.expected_version {
            return Err(CorvusError::Upgraded {
                database: version,
                expected: self.expected_version,
            });
        }
        let wtx = WriteTx { tx };
        let out = f(&wtx)?;
        wtx.tx.commit().db()?;
        Ok(out)
    }

    /// Checkpoints the WAL into the main file and truncates it (on shutdown). Not a write transaction.
    pub fn checkpoint(&self) -> Result<()> {
        let conn = lock(&self.writer);
        let (busy, _, _): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .db()?;
        if busy != 0 {
            return Err(CorvusError::Busy);
        }
        Ok(())
    }

    /// Write-transaction timings since this handle opened (or since [`Database::reset_write_stats`]).
    pub fn write_stats(&self) -> WriteStats {
        WriteStats {
            transactions: self.stats.transactions.load(Ordering::Relaxed),
            max_wait_us: self.stats.max_wait_us.load(Ordering::Relaxed),
            max_hold_us: self.stats.max_hold_us.load(Ordering::Relaxed),
        }
    }

    pub fn reset_write_stats(&self) {
        self.stats.transactions.store(0, Ordering::Relaxed);
        self.stats.max_wait_us.store(0, Ordering::Relaxed);
        self.stats.max_hold_us.store(0, Ordering::Relaxed);
    }

    /// Runs `f` in a deferred transaction on a read-only (`query_only`) connection.
    pub fn read<T>(&self, f: impl FnOnce(&ReadTx<'_>) -> Result<T>) -> Result<T> {
        let pooled = lock(&self.readers).pop();
        let mut conn = match pooled {
            Some(c) => c,
            None => open_conn(&self.path, Role::Reader, BUSY_TIMEOUT_MS).map_err(CorvusError::from)?,
        };
        let out = read_on(&mut conn, f);
        let mut pool = lock(&self.readers);
        if pool.len() < MAX_IDLE_READERS {
            pool.push(conn);
        }
        out
    }

    /// Writes a consistent copy of the database to `dest` with `VACUUM INTO` (safe while other processes
    /// write). `dest` must not exist; it is created with mode `0600`.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let conn = open_conn(&self.path, Role::Reader, BUSY_TIMEOUT_MS).map_err(CorvusError::from)?;
        // A query_only connection may not VACUUM; this short-lived maintenance connection only reads the
        // source and writes the new file.
        conn.pragma_update(None, "query_only", false).db()?;
        vacuum_into(&conn, dest)
    }
}

/// Hermod's dedicated read connection: `PRAGMA data_version` (which changes when any *other* connection
/// commits) plus read transactions for the hint cursor.
pub struct HintReader {
    conn: Connection,
}

impl fmt::Debug for HintReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HintReader")
    }
}

impl HintReader {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            conn: open_conn(path, Role::Reader, BUSY_TIMEOUT_MS).map_err(CorvusError::from)?,
        })
    }

    pub fn data_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0)).db()
    }

    pub fn read<T>(&mut self, f: impl FnOnce(&ReadTx<'_>) -> Result<T>) -> Result<T> {
        read_on(&mut self.conn, f)
    }
}

/// Records a write transaction's wait and hold time when it ends (commit, rollback or error).
struct HoldTimer<'a> {
    stats: &'a WriteCounters,
    began: Instant,
    waited: Duration,
}

impl Drop for HoldTimer<'_> {
    fn drop(&mut self) {
        let held = self.began.elapsed();
        let us = |d: Duration| u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        self.stats.transactions.fetch_add(1, Ordering::Relaxed);
        self.stats
            .max_wait_us
            .fetch_max(us(self.waited), Ordering::Relaxed);
        self.stats.max_hold_us.fetch_max(us(held), Ordering::Relaxed);
        if held > SLOW_WRITE {
            tracing::warn!(
                duration_ms = u64::try_from(held.as_millis()).unwrap_or(u64::MAX),
                "slow write transaction"
            );
        }
    }
}

/// A database opened read-only at the OS level, for the hook. It never migrates and has no write method.
pub struct ReadOnlyDatabase {
    conn: Mutex<Connection>,
}

impl fmt::Debug for ReadOnlyDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReadOnlyDatabase")
    }
}

impl ReadOnlyDatabase {
    /// Opens an existing, fully migrated database. Fails if it is missing or at another schema version.
    pub fn open(path: &Path, busy_timeout_ms: u32) -> std::result::Result<Self, OpenError> {
        if !path.is_file() {
            return Err(OpenError::Missing(path.to_owned()));
        }
        let conn = open_conn(path, Role::ReadOnly, busy_timeout_ms)?;
        let v = user_version(&conn).map_err(|e| OpenError::Sql(sql_err(e)))?;
        if v != SCHEMA_VERSION {
            return Err(OpenError::SchemaMismatch {
                database: v,
                binary: SCHEMA_VERSION,
            });
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn read<T>(&self, f: impl FnOnce(&ReadTx<'_>) -> Result<T>) -> Result<T> {
        read_on(&mut lock(&self.conn), f)
    }
}

fn read_on<T>(conn: &mut Connection, f: impl FnOnce(&ReadTx<'_>) -> Result<T>) -> Result<T> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .db()?;
    let rtx = ReadTx { tx };
    let out = f(&rtx);
    // Nothing to commit; ending the snapshot either way is fine.
    rtx.tx.rollback().db()?;
    out
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// Creates the parent directory (`0700`) and the database file (`0600`) if they do not exist.
fn prepare_file(path: &Path) -> std::result::Result<(), OpenError> {
    let io = |error| OpenError::Io {
        path: path.to_owned(),
        error,
    };
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent).map_err(io)?;
    }
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(io(e)),
    }
}

fn open_conn(path: &Path, role: Role, busy_timeout_ms: u32) -> std::result::Result<Connection, OpenError> {
    let flags = match role {
        Role::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        _ => OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    };
    let conn = Connection::open_with_flags(path, flags).map_err(|e| OpenError::Sql(sql_err(e)))?;
    apply_pragmas(&conn, role, busy_timeout_ms)?;
    Ok(conn)
}

fn apply_pragmas(conn: &Connection, role: Role, busy_timeout_ms: u32) -> std::result::Result<(), OpenError> {
    let sql = |e| OpenError::Sql(sql_err(e));
    conn.busy_timeout(Duration::from_millis(u64::from(busy_timeout_ms)))
        .map_err(sql)?;

    let journal: String = if role == Role::ReadOnly {
        conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(sql)?
    } else {
        // Switching a fresh file to WAL needs a brief exclusive lock; several processes opening a new database
        // at once can see SQLITE_BUSY here despite the busy handler, so retry within the busy budget.
        let mut tries = 0;
        loop {
            match conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
                Ok(m) => break m,
                Err(e) if sql_err_is_busy(&e) && tries < 200 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(sql(e)),
            }
        }
    };
    check("journal_mode", "wal", &journal.to_ascii_lowercase())?;

    conn.pragma_update(None, "foreign_keys", true).map_err(sql)?;
    conn.pragma_update(None, "trusted_schema", false).map_err(sql)?;
    if role != Role::ReadOnly {
        conn.pragma_update(None, "synchronous", "FULL").map_err(sql)?;
    }
    if role != Role::Writer {
        conn.pragma_update(None, "query_only", true).map_err(sql)?;
    }

    let get = |p: &str| {
        conn.query_row(&format!("PRAGMA {p}"), [], |r| r.get::<_, i64>(0))
            .map_err(sql)
    };
    check("foreign_keys", "1", &get("foreign_keys")?.to_string())?;
    check("trusted_schema", "0", &get("trusted_schema")?.to_string())?;
    check(
        "busy_timeout",
        &busy_timeout_ms.to_string(),
        &get("busy_timeout")?.to_string(),
    )?;
    if role != Role::ReadOnly {
        check("synchronous", "2", &get("synchronous")?.to_string())?;
    }
    let query_only = if role == Role::Writer { "0" } else { "1" };
    check("query_only", query_only, &get("query_only")?.to_string())?;
    Ok(())
}

fn sql_err_is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn check(pragma: &'static str, expected: &str, actual: &str) -> std::result::Result<(), OpenError> {
    if expected == actual {
        Ok(())
    } else {
        Err(OpenError::Pragma {
            pragma,
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

/// `VACUUM INTO dest`, creating `dest` first with mode `0600`. Refuses an existing `dest`.
pub(crate) fn vacuum_into(conn: &Connection, dest: &Path) -> Result<()> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dest)
    {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(CorvusError::Conflict(format!(
                "{} already exists",
                dest.display()
            )));
        }
        Err(e) => return Err(CorvusError::Internal(format!("{}: {e}", dest.display()))),
    }
    let dest_str = dest
        .to_str()
        .ok_or_else(|| CorvusError::Validation("backup path must be valid UTF-8".to_owned()))?;
    if let Err(e) = conn.execute("VACUUM INTO ?1", [dest_str]) {
        let _ = std::fs::remove_file(dest);
        return Err(sql_err(e));
    }
    Ok(())
}

/// Live pragma values of a connection, for tests and `health`.
#[doc(hidden)]
pub fn pragma_snapshot(r: &impl Reader) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for p in [
        "journal_mode",
        "foreign_keys",
        "synchronous",
        "busy_timeout",
        "trusted_schema",
        "query_only",
    ] {
        let v: rusqlite::types::Value = r
            .sql()
            .query_row(&format!("PRAGMA {p}"), [], |row| row.get(0))
            .db()?;
        let s = match v {
            rusqlite::types::Value::Integer(i) => i.to_string(),
            rusqlite::types::Value::Text(t) => t,
            other => format!("{other:?}"),
        };
        out.push((p.to_owned(), s));
    }
    Ok(out)
}
