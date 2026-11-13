//! Background work for long-lived processes ([system/10](../../../spec/system/10-config-ops.md#housekeeping)):
//! the housekeeping pass, the loop that runs it, and the background-leader lock that keeps it to one process.
//!
//! Every step is an ordinary short write transaction (D0017), each inside its own error boundary that also
//! catches panics: a failing step is logged with its name and the next step still runs. The loop never exits
//! while the process runs (P8).

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvus_domain::{CorvusError, Result};
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

use std::sync::Arc;

use corvus_domain::SourceId;

use crate::clock::format_rfc3339;
use crate::services::Services;
use crate::sources::Sources;
use crate::store::hints;

/// Hint retention ([defaults #13](../../../spec/system/11-defaults.md)).
pub const HINTS_KEPT: i64 = 1_000;
pub const FIRST_PASS_AFTER: Duration = Duration::from_secs(5);
pub const PASS_INTERVAL: Duration = Duration::from_secs(60);
pub const LEADER_RETRY: Duration = Duration::from_secs(30);

/// One housekeeping step: a name and an operation returning how many things it handled.
pub type Step = (&'static str, fn(&Services) -> Result<usize>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum StepOutcome {
    Ok { count: usize },
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepReport {
    pub step: &'static str,
    #[serde(flatten)]
    pub outcome: StepOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PassReport {
    pub steps: Vec<StepReport>,
}

impl PassReport {
    pub fn count(&self, step: &str) -> Option<usize> {
        self.steps
            .iter()
            .find(|s| s.step == step)
            .and_then(|s| match s.outcome {
                StepOutcome::Ok { count } => Some(count),
                _ => None,
            })
    }

    pub fn failed(&self) -> Vec<&'static str> {
        self.steps
            .iter()
            .filter(|s| matches!(s.outcome, StepOutcome::Failed { .. }))
            .map(|s| s.step)
            .collect()
    }
}

fn expire_leases(s: &Services) -> Result<usize> {
    Ok(s.huginn().expire_leases()?.expired)
}

fn end_stale_sessions(s: &Services) -> Result<usize> {
    Ok(s.sessions().end_stale()?.ended.len())
}

fn prune_hints(s: &Services) -> Result<usize> {
    s.db().write(|tx| hints::prune(tx, HINTS_KEPT))
}

/// The steps of a pass, in order.
pub const STEPS: &[Step] = &[
    ("expire_leases", expire_leases),
    ("end_stale_sessions", end_stale_sessions),
    ("prune_hints", prune_hints),
];

/// One housekeeping pass. A plain function: callable from the loop, tests and admin commands.
pub fn housekeeping_pass(s: &Services) -> PassReport {
    housekeeping_pass_with(s, STEPS)
}

/// A pass over explicit steps (tests inject failing ones).
pub fn housekeeping_pass_with(s: &Services, steps: &[Step]) -> PassReport {
    let mut out = Vec::with_capacity(steps.len());
    for &(name, step) in steps {
        let outcome = if name == "end_stale_sessions" && s.settings().auto_end_ms <= 0 {
            StepOutcome::Skipped {
                reason: "CORVUS_AUTO_END_SECONDS is 0".into(),
            }
        } else {
            match catch_unwind(AssertUnwindSafe(|| step(s))) {
                Ok(Ok(count)) => StepOutcome::Ok { count },
                Ok(Err(e)) => {
                    tracing::error!(step = name, error = %e, "housekeeping step failed; continuing");
                    StepOutcome::Failed { error: e.to_string() }
                }
                Err(panic) => {
                    let msg = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "panic".into());
                    tracing::error!(step = name, panic = %msg, "housekeeping step panicked; continuing");
                    StepOutcome::Failed {
                        error: format!("panicked: {msg}"),
                    }
                }
            }
        };
        out.push(StepReport { step: name, outcome });
    }
    PassReport { steps: out }
}

// ---------- background leader ----------

/// Holder of the exclusive lock on `<state dir>/run/leader.lock`. The OS releases the lock if the process dies.
#[derive(Debug)]
pub struct BackgroundLeader {
    _lock: File,
    info_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct LeaderInfo {
    pub pid: u32,
    pub kind: String,
    pub since: String,
}

impl BackgroundLeader {
    /// Tries to become the leader without blocking. `Ok(None)` when another process holds the lock.
    pub fn try_acquire(state_dir: &Path, kind: &str) -> std::io::Result<Option<Self>> {
        let run = state_dir.join("run");
        crate::config::ensure_private_dir(&run)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(run.join("leader.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        let info = LeaderInfo {
            pid: std::process::id(),
            kind: kind.to_owned(),
            since: format_rfc3339(crate::clock::Clock::now_ms(&crate::clock::SystemClock)),
        };
        let info_path = run.join("leader.json");
        let tmp = run.join(format!(".leader.json.{}", std::process::id()));
        {
            let mut f = File::create(&tmp)?;
            f.write_all(serde_json::to_string(&info).unwrap_or_default().as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &info_path)?;
        Ok(Some(Self {
            _lock: lock,
            info_path,
        }))
    }

    /// The current holder, as recorded in `leader.json` (may be stale if the holder crashed).
    pub fn read_info(state_dir: &Path) -> Option<LeaderInfo> {
        let text = std::fs::read_to_string(state_dir.join("run/leader.json")).ok()?;
        serde_json::from_str(&text).ok()
    }
}

impl Drop for BackgroundLeader {
    fn drop(&mut self) {
        // Only remove the record if it is still ours.
        if let Ok(text) = std::fs::read_to_string(&self.info_path)
            && serde_json::from_str::<LeaderInfo>(&text).is_ok_and(|i| i.pid == std::process::id())
        {
            let _ = std::fs::remove_file(&self.info_path);
        }
    }
}

/// Timings and steps of the background loop (tests shrink the timings and inject steps).
#[derive(Debug, Clone, Copy)]
pub struct LoopTimings {
    pub first_pass: Duration,
    pub interval: Duration,
    pub leader_retry: Duration,
    pub steps: &'static [Step],
}

impl Default for LoopTimings {
    fn default() -> Self {
        Self {
            first_pass: FIRST_PASS_AFTER,
            interval: PASS_INTERVAL,
            leader_retry: LEADER_RETRY,
            steps: STEPS,
        }
    }
}

/// Runs until `shutdown` turns true: become leader (retrying every 30 s), then run a pass after 5 s and every
/// 60 s, skipping ticks that fall due while a pass is still running. `on_pass` sees every report.
pub async fn run_background(
    services: Services,
    state_dir: PathBuf,
    kind: &'static str,
    timings: LoopTimings,
    shutdown: watch::Receiver<bool>,
    on_pass: impl Fn(&PassReport) + Send + 'static,
) {
    run_background_with_sources(services, state_dir, kind, timings, shutdown, on_pass, None).await;
}

/// One poll loop per enabled source, for as long as this process is the leader: a pass after `first`, then every
/// source's own interval. A failed pass is logged and the loop continues (P8).
async fn poll_source(
    sources: Arc<Sources>,
    id: SourceId,
    first: Duration,
    every: Duration,
    mut stop: watch::Receiver<bool>,
) {
    let mut next = tokio::time::Instant::now() + first;
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(next) => {}
            changed = stop.changed() => if changed.is_err() || *stop.borrow() { return; }
        }
        if *stop.borrow() {
            return;
        }
        match sources.sync(id).await {
            Ok(r) if r.ok => tracing::info!(source = id.as_str(), stats = ?r.stats, "source sync"),
            Ok(r) => tracing::warn!(source = id.as_str(), error = ?r.error, "source sync failed; will retry"),
            Err(e) => {
                tracing::warn!(source = id.as_str(), error = %e, "source sync could not run; will retry")
            }
        }
        next = tokio::time::Instant::now() + every;
    }
}

/// [`run_background`] plus the work-source poll loops, which also run only in the leader.
pub async fn run_background_with_sources(
    services: Services,
    state_dir: PathBuf,
    kind: &'static str,
    timings: LoopTimings,
    mut shutdown: watch::Receiver<bool>,
    on_pass: impl Fn(&PassReport) + Send + 'static,
    sources: Option<Arc<Sources>>,
) {
    let leader = loop {
        if *shutdown.borrow() {
            return;
        }
        let (dir, k) = (state_dir.clone(), kind);
        match tokio::task::spawn_blocking(move || BackgroundLeader::try_acquire(&dir, k)).await {
            Ok(Ok(Some(l))) => {
                tracing::info!(kind, "became the background leader");
                break l;
            }
            Ok(Ok(None)) => tracing::debug!("another process is the background leader; will retry"),
            Ok(Err(e)) => tracing::warn!(error = %e, "could not take the leader lock; will retry"),
            Err(e) => tracing::warn!(error = %e, "leader lock task failed; will retry"),
        }
        tokio::select! {
            () = tokio::time::sleep(timings.leader_retry) => {}
            changed = shutdown.changed() => {
                // A dropped sender means the owner is gone: stop.
                if changed.is_err() {
                    return;
                }
            }
        }
    };

    let polls: Vec<tokio::task::JoinHandle<()>> = sources
        .map(|s| {
            s.enabled()
                .into_iter()
                .map(|src| {
                    let every = src.poll_interval();
                    tokio::spawn(poll_source(
                        s.clone(),
                        src.id(),
                        timings.first_pass,
                        every,
                        shutdown.clone(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut ticks =
        tokio::time::interval_at(tokio::time::Instant::now() + timings.first_pass, timings.interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            changed = shutdown.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
        if *shutdown.borrow() {
            break;
        }
        let s = services.clone();
        let steps = timings.steps;
        match tokio::task::spawn_blocking(move || housekeeping_pass_with(&s, steps)).await {
            Ok(report) => {
                tracing::info!(report = %serde_json::to_string(&report).unwrap_or_default(), "housekeeping pass");
                on_pass(&report);
            }
            Err(e) => tracing::error!(error = %e, "housekeeping pass task failed; continuing"),
        }
    }
    for p in polls {
        p.abort();
    }
    drop(leader);
}

/// Checkpoints the WAL into the database file (on shutdown).
pub fn checkpoint(s: &Services) -> Result<()> {
    s.db()
        .checkpoint()
        .map_err(|e| CorvusError::Internal(format!("checkpoint: {e}")))
}
