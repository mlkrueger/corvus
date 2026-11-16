//! Services own every transaction ([system/03](../../../../spec/system/03-services.md)). One method = one
//! transaction: mutations through [`Database::write`] (`BEGIN IMMEDIATE`), reads through [`Database::read`].
//! Validation happens before the transaction; nothing slow happens inside it.

pub mod export;
pub mod huginn;
pub mod muninn;
pub mod overview;
pub mod sessions;
pub(crate) mod views;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use corvus_domain::{AgentName, CorvusError, Result, SessionId, TaskId};

use crate::clock::Clock;
use crate::config::Config;
use crate::db::{Database, WriteTx};
use crate::store;

pub use export::Export;
pub use huginn::Huginn;
pub use muninn::Muninn;
pub use overview::OverviewService;
pub use sessions::Sessions;

/// Durations the services need, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub lease_ms: i64,
    pub active_ms: i64,
    pub idle_ms: i64,
    /// 0 disables auto-end.
    pub auto_end_ms: i64,
}

impl Settings {
    /// For callers whose one operation reads no duration (the hook's conversation carry-over).
    pub const UNUSED: Self = Self {
        lease_ms: 0,
        active_ms: 0,
        idle_ms: 0,
        auto_end_ms: 0,
    };

    pub fn from_config(c: &Config) -> Self {
        let ms = |s: u64| i64::try_from(s).unwrap_or(i64::MAX / 1000) * 1000;
        Self {
            lease_ms: ms(c.lease_seconds),
            active_ms: ms(c.active_seconds),
            idle_ms: ms(c.idle_seconds),
            auto_end_ms: ms(c.auto_end_seconds),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lease_ms: 900_000,
            active_ms: 120_000,
            idle_ms: 900_000,
            auto_end_ms: 3_600_000,
        }
    }
}

/// The service layer for one process: a database handle, a clock and settings.
#[derive(Debug, Clone)]
pub struct Services {
    db: Arc<Database>,
    clock: Arc<dyn Clock>,
    settings: Settings,
    operator_name: AgentName,
    export_repo: Option<PathBuf>,
}

impl Services {
    pub fn new(db: Arc<Database>, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        Self {
            db,
            clock,
            settings,
            operator_name: AgentName::parse("operator").expect("valid literal"),
            export_repo: None,
        }
    }

    /// Services configured from `Config` (operator name, export repository, durations).
    pub fn from_config(db: Arc<Database>, clock: Arc<dyn Clock>, config: &Config) -> Self {
        Self::new(db, clock, Settings::from_config(config))
            .with_operator(config.operator_name.clone())
            .with_export_repo(config.export_repo.clone())
    }

    pub fn with_operator(mut self, name: AgentName) -> Self {
        self.operator_name = name;
        self
    }

    pub fn with_export_repo(mut self, repo: Option<PathBuf>) -> Self {
        self.export_repo = repo;
        self
    }

    pub fn operator_name(&self) -> &AgentName {
        &self.operator_name
    }

    pub fn export_repo(&self) -> Option<&Path> {
        self.export_repo.as_deref()
    }

    pub fn muninn(&self) -> Muninn<'_> {
        Muninn { s: self }
    }

    pub fn overview(&self) -> OverviewService<'_> {
        OverviewService { s: self }
    }

    pub fn export(&self) -> Export<'_> {
        Export { s: self }
    }

    pub fn sessions(&self) -> Sessions<'_> {
        Sessions { s: self }
    }

    pub fn huginn(&self) -> Huginn<'_> {
        Huginn { s: self }
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now_ms()
    }
}

// ---------- shared validation and primitives ----------

pub(crate) fn agent(field: &str, s: &str) -> Result<AgentName> {
    AgentName::parse(s.trim()).map_err(|e| CorvusError::Validation(format!("{field}: {}", strip(&e))))
}

pub(crate) fn session_id(s: &str) -> Result<SessionId> {
    SessionId::parse(s.trim()).map_err(|_| CorvusError::Validation("session_id: must be a UUID".into()))
}

pub(crate) fn task_id(field: &str, s: &str) -> Result<TaskId> {
    TaskId::parse(s.trim()).map_err(|_| CorvusError::Validation(format!("{field}: must be a UUID")))
}

/// Optional caller-chosen idempotency key: 1-200 printable characters.
pub(crate) fn idempotency_key(k: &Option<String>) -> Result<Option<String>> {
    match k.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(k) if k.chars().count() <= 200 && !k.chars().any(char::is_control) => Ok(Some(k.to_owned())),
        Some(_) => Err(CorvusError::Validation(
            "idempotency_key: at most 200 printable characters".into(),
        )),
    }
}

/// A short single-line label (pool, client name): 1-`max` printable characters.
pub(crate) fn label(field: &str, s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > max || s.chars().any(char::is_control) {
        return Err(CorvusError::Validation(format!(
            "{field}: 1-{max} printable characters"
        )));
    }
    Ok(s.to_owned())
}

/// Optional free text with a character cap.
pub(crate) fn text(field: &str, s: &Option<String>, max: usize) -> Result<Option<String>> {
    match s {
        None => Ok(None),
        Some(t) if t.chars().count() <= max => Ok(Some(t.clone())),
        Some(_) => Err(CorvusError::Validation(format!(
            "{field}: at most {max} characters"
        ))),
    }
}

fn strip(e: &CorvusError) -> String {
    match e {
        CorvusError::Validation(m) => m.clone(),
        other => other.to_string(),
    }
}

/// The heartbeat every call with a session id begins with (system/03 rule 3). Returns the session's agent.
pub(crate) fn touch(tx: &WriteTx<'_>, id: &SessionId, now: i64) -> Result<AgentName> {
    match store::sessions::touch(tx, &id.to_string(), now)? {
        Some(a) => AgentName::parse(&a),
        None => Err(CorvusError::SessionNotActive),
    }
}
