//! The wake supervisor ([system/06](../../../../spec/system/06-delivery-helpers.md#wake-supervisor-corvus-watch-agent)).
//!
//! Three modes share one loop: **print** (wait for unread mail, return it), **exec** (run a command per batch of
//! new unread messages) and **adapter** (run the agent's adapter per batch of new mail that wakes — direct mail
//! and `wake` broadcasts; the tool gets no message text).
//! The supervisor never acks. Delivery bookkeeping is a *set* of message ids in
//! `$STATE/watch/<agent>/delivered.json`; one watcher per agent is enforced by an exclusive file lock.
//!
//! Crash semantics: a crash between the command succeeding and the set being persisted re-delivers that batch
//! after restart. Duplicate delivery is accepted; lost delivery is not.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use corvus_domain::{Adapter, AgentName, CorvusError, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::api::{MessageView, ReadInbox};
use crate::config::ensure_private_dir;
use crate::delivery::adapters::kill_group;
use crate::delivery::bindings::write_private_atomic;
use crate::delivery::waiter::wakes;
use crate::facade::Corvus;
use crate::hermod::Subscription;

/// Default command timeout ([system/06](../../../../spec/system/06-delivery-helpers.md#wake-supervisor-corvus-watch-agent)).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const PAGE: u32 = 500;
const POLL: Duration = Duration::from_millis(10);

/// Re-check and backoff schedule. Injectable so tests can shrink it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchTimings {
    /// Re-read the inbox at least this often even without a hint.
    pub recheck: Duration,
    /// Delay after the first failure; doubles on each further failure.
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
}

impl Default for WatchTimings {
    fn default() -> Self {
        Self {
            recheck: crate::hermod::RECHECK_INTERVAL,
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(300),
        }
    }
}

impl WatchTimings {
    /// The delay after the `failures`-th consecutive failure, counting from 0: base, 2*base, 4*base … cap.
    pub fn backoff(&self, failures: u32) -> Duration {
        self.backoff_base
            .saturating_mul(1u32 << failures.min(20))
            .min(self.backoff_cap)
    }
}

/// What to run for a batch.
#[derive(Debug, Clone)]
pub enum Wake {
    /// `corvus watch --exec`: the command gets the messages (stdin and a file).
    Exec { argv: Vec<String>, timeout: Duration },
    /// The agent's adapter: the tool gets only the fixed note.
    Adapter {
        adapter: Adapter,
        conversation: Option<String>,
        timeout: Duration,
    },
}

impl Wake {
    pub fn exec(argv: Vec<String>, timeout: Duration) -> Result<Self> {
        if argv.first().is_none_or(|p| p.trim().is_empty()) {
            return Err(CorvusError::Validation("--exec needs a command".into()));
        }
        Ok(Self::Exec { argv, timeout })
    }

    /// `timeout` overrides the adapter's own `timeout_seconds`.
    pub fn adapter(adapter: Adapter, conversation: Option<String>, timeout: Option<Duration>) -> Self {
        let timeout = timeout.unwrap_or(Duration::from_secs(adapter.timeout_seconds));
        Self::Adapter {
            adapter,
            conversation,
            timeout,
        }
    }
}

/// One command to run.
#[derive(Debug)]
struct Invocation {
    argv: Vec<String>,
    env: Vec<(String, String)>,
    /// `None` = null stdin.
    stdin: Option<Vec<u8>>,
    timeout: Duration,
}

fn inbox_topics(agent: &AgentName) -> [String; 2] {
    [format!("inbox:{agent}"), "inbox:*".to_owned()]
}

/// `$STATE/watch/<agent>`.
pub fn watch_dir(state_dir: &Path, agent: &AgentName) -> PathBuf {
    state_dir.join("watch").join(agent.as_str())
}

/// The exclusive per-agent watcher lock. Held for as long as the value lives (the OS releases it on exit).
#[derive(Debug)]
pub struct WatchLock {
    _file: File,
}

impl WatchLock {
    /// A second watcher for the same agent gets `Conflict`.
    pub fn acquire(state_dir: &Path, agent: &AgentName) -> Result<Self> {
        let dir = watch_dir(state_dir, agent);
        let io = |e: std::io::Error| CorvusError::Internal(format!("{}: {e}", dir.display()));
        ensure_private_dir(&dir).map_err(io)?;
        let path = dir.join("lock");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|e| CorvusError::Internal(format!("{}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(CorvusError::Conflict(format!(
                    "another watcher is already running for {agent}"
                )));
            }
            Err(TryLockError::Error(e)) => {
                return Err(CorvusError::Internal(format!("{}: {e}", path.display())));
            }
        }
        // Informational (for `corvus status`); the lock itself is the truth.
        if file.set_len(0).is_ok() {
            let _ = write!(file, "{}", std::process::id());
        }
        Ok(Self { _file: file })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeliveredFile {
    #[serde(default)]
    ids: BTreeSet<i64>,
}

fn load_delivered(path: &Path) -> BTreeSet<i64> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<DeliveredFile>(&text) {
            Ok(f) => f.ids,
            // Unreadable bookkeeping means possible duplicates, never lost mail.
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "delivered set is malformed; starting empty");
                BTreeSet::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => {
            tracing::warn!(file = %path.display(), error = %e, "cannot read delivered set; starting empty");
            BTreeSet::new()
        }
    }
}

fn persist_delivered(path: &Path, ids: &BTreeSet<i64>) {
    let bytes = serde_json::to_vec(&DeliveredFile { ids: ids.clone() }).unwrap_or_default();
    if let Err(e) = write_private_atomic(path, &bytes) {
        // The in-memory set still prevents re-delivery in this process; a restart may duplicate.
        tracing::warn!(file = %path.display(), error = %e, "cannot persist the delivered set");
    }
}

/// All unread messages for `agent`, oldest first (paged; unread is a query, not a cursor).
async fn read_unread(corvus: &Corvus, agent: &AgentName) -> Result<Vec<MessageView>> {
    let mut all = Vec::new();
    let mut after_id = 0;
    loop {
        let req = ReadInbox {
            agent: agent.to_string(),
            limit: Some(PAGE),
            after_id: Some(after_id),
        };
        let page = corvus.run(move |s| s.muninn().read_inbox(&req)).await?;
        let n = page.len();
        if let Some(last) = page.last() {
            after_id = last.id;
        }
        all.extend(page);
        if n < PAGE as usize {
            return Ok(all);
        }
    }
}

/// Resolves when `stop` is set. Never resolves if the sender is gone.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    if stop.wait_for(|s| *s).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Waits up to `d` (or until a hint, if given). Returns true if the watcher was told to stop.
async fn pause(stop: &mut watch::Receiver<bool>, d: Duration, sub: Option<&Subscription>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(d) => {}
        () = async {
            match sub {
                Some(s) => s.notified().await,
                None => std::future::pending().await,
            }
        } => {}
        () = stopped(stop) => {}
    }
    *stop.borrow()
}

/// Print mode: blocks until `agent` has unread mail (subscribe first, then read, then wait for a hint or the
/// re-check) and returns it. Never acks. `None` if stopped first.
pub async fn wait_for_unread(
    corvus: &Corvus,
    agent: &AgentName,
    recheck: Duration,
    mut stop: watch::Receiver<bool>,
) -> Result<Option<Vec<MessageView>>> {
    loop {
        let sub = corvus.hermod().subscribe(&inbox_topics(agent)).await?;
        let unread = read_unread(corvus, agent).await?;
        if !unread.is_empty() {
            return Ok(Some(unread));
        }
        if pause(&mut stop, recheck, Some(&sub)).await {
            return Ok(None);
        }
    }
}

/// Exec and adapter modes: loops until `stop` is set. Takes the per-agent lock first (`Conflict` if another
/// watcher holds it). Errors while running are logged and retried with backoff; only the lock or an unusable
/// state directory fail the call.
pub async fn watch_loop(
    corvus: &Corvus,
    state_dir: &Path,
    agent: &AgentName,
    wake: Wake,
    timings: WatchTimings,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let _lock = WatchLock::acquire(state_dir, agent)?;
    let dir = watch_dir(state_dir, agent);
    let delivered_path = dir.join("delivered.json");
    let batch_path = dir.join("batch.json");
    let mut delivered = load_delivered(&delivered_path);
    // The batch being retried after a failure.
    let mut pending: Option<Vec<MessageView>> = None;
    let mut failures: u32 = 0;
    let mut read_errors: u32 = 0;
    let mode = match &wake {
        Wake::Exec { .. } => "exec",
        Wake::Adapter { .. } => "adapter",
    };
    tracing::info!(agent = %agent, mode, delivered = delivered.len(), "watcher started");

    loop {
        if *stop.borrow() {
            return Ok(());
        }
        // Subscribe first, then read, then wait.
        let sub = match corvus.hermod().subscribe(&inbox_topics(agent)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(agent = %agent, error = %e, "cannot subscribe; backing off");
                if pause(&mut stop, timings.backoff(read_errors), None).await {
                    return Ok(());
                }
                read_errors = read_errors.saturating_add(1);
                continue;
            }
        };
        let unread = match read_unread(corvus, agent).await {
            Ok(u) => {
                read_errors = 0;
                u
            }
            Err(e) => {
                tracing::warn!(agent = %agent, error = %e, "cannot read the inbox; backing off");
                if pause(&mut stop, timings.backoff(read_errors), None).await {
                    return Ok(());
                }
                read_errors = read_errors.saturating_add(1);
                continue;
            }
        };
        let unread_ids: BTreeSet<i64> = unread.iter().map(|m| m.id).collect();

        // Prune the set to ids that are still unread (acked messages never come back).
        let before = delivered.len();
        delivered.retain(|id| unread_ids.contains(id));
        if delivered.len() != before {
            persist_delivered(&delivered_path, &delivered);
        }

        // Retry the same batch (minus anything acked meanwhile); otherwise take the new messages.
        let mut batch: Vec<MessageView> = pending
            .take()
            .map(|p| p.into_iter().filter(|m| unread_ids.contains(&m.id)).collect())
            .unwrap_or_default();
        if batch.is_empty() {
            failures = 0;
            batch = unread
                .into_iter()
                .filter(|m| !delivered.contains(&m.id))
                .collect();
        }
        // An adapter starts a model turn, so only mail that wakes counts (defaults row 62). Other broadcasts
        // stay unread and are seen at the agent's next turn.
        if matches!(wake, Wake::Adapter { .. }) {
            batch.retain(|m| wakes(m, agent));
        }
        if batch.is_empty() {
            if pause(&mut stop, timings.recheck, Some(&sub)).await {
                return Ok(());
            }
            continue;
        }
        drop(sub);

        let outcome = match build_invocation(&wake, agent, state_dir, &batch_path, &batch) {
            Ok(inv) => {
                let stop_rx = stop.clone();
                match tokio::task::spawn_blocking(move || run_invocation(&inv, &stop_rx)).await {
                    Ok(r) => r,
                    Err(e) => Err(format!("runner task failed: {e}")),
                }
            }
            Err(msg) => Err(msg),
        };
        match outcome {
            Ok(()) => {
                tracing::info!(agent = %agent, count = batch.len(), "batch delivered");
                delivered.extend(batch.iter().map(|m| m.id));
                delivered.retain(|id| unread_ids.contains(id));
                persist_delivered(&delivered_path, &delivered);
                failures = 0;
            }
            Err(msg) => {
                let delay = timings.backoff(failures);
                tracing::warn!(agent = %agent, mode, count = batch.len(), attempt = failures + 1,
                    retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX), error = %msg,
                    "delivery failed; will retry the same batch");
                pending = Some(batch);
                failures = failures.saturating_add(1);
                if pause(&mut stop, delay, None).await {
                    return Ok(());
                }
            }
        }
    }
}

fn build_invocation(
    wake: &Wake,
    agent: &AgentName,
    state_dir: &Path,
    batch_path: &Path,
    batch: &[MessageView],
) -> std::result::Result<Invocation, String> {
    match wake {
        Wake::Exec { argv, timeout } => {
            let json = serde_json::to_vec(batch).map_err(|e| format!("cannot encode the batch: {e}"))?;
            write_private_atomic(batch_path, &json)
                .map_err(|e| format!("cannot write {}: {e}", batch_path.display()))?;
            Ok(Invocation {
                argv: argv.clone(),
                env: vec![
                    ("CORVUS_WAKE_AGENT".into(), agent.to_string()),
                    (
                        "CORVUS_WAKE_STATE_DIR".into(),
                        state_dir.to_string_lossy().into_owned(),
                    ),
                    (
                        "CORVUS_WAKE_MESSAGES_FILE".into(),
                        batch_path.to_string_lossy().into_owned(),
                    ),
                ],
                stdin: Some(json),
                timeout: *timeout,
            })
        }
        // No stdin, no messages file, no message text anywhere: only the fixed note via `{note}`.
        Wake::Adapter {
            adapter,
            conversation,
            timeout,
        } => Ok(Invocation {
            argv: adapter.render_wake(
                agent,
                conversation.as_deref(),
                batch.len(),
                &state_dir.to_string_lossy(),
            ),
            env: Vec::new(),
            stdin: None,
            timeout: *timeout,
        }),
    }
}

/// Spawns the command (argument vector, own process group), feeds stdin, and waits up to the timeout, killing
/// the whole group on timeout or stop. `Ok` only for exit status 0.
fn run_invocation(inv: &Invocation, stop: &watch::Receiver<bool>) -> std::result::Result<(), String> {
    let program = &inv.argv[0];
    let mut cmd = Command::new(program);
    cmd.args(&inv.argv[1..])
        .envs(inv.env.iter().cloned())
        .process_group(0);
    // The command's stdout goes to our stderr so it never mixes into our JSON stdout.
    match std::io::stderr().as_fd().try_clone_to_owned().map(Stdio::from) {
        Ok(out) => cmd.stdout(out),
        Err(_) => cmd.stdout(Stdio::null()),
    };
    cmd.stdin(if inv.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("tool not installed: {program:?} was not found on PATH")
        } else {
            format!("cannot start {program:?}: {e}")
        }
    })?;
    let pid = child.id();
    if let (Some(mut stdin), Some(data)) = (child.stdin.take(), inv.stdin.clone()) {
        // A thread, so a command that never reads stdin cannot block us; the timeout still applies.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let deadline = Instant::now() + inv.timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("{program:?} {status}")),
            Ok(None) => {}
            Err(e) => {
                kill_group(pid);
                let _ = child.wait();
                return Err(format!("cannot wait for {program:?}: {e}"));
            }
        }
        if *stop.borrow() {
            kill_group(pid);
            let _ = child.wait();
            return Err("watcher stopping".into());
        }
        if Instant::now() >= deadline {
            kill_group(pid);
            let _ = child.wait();
            return Err(format!(
                "{program:?} timed out after {:.1}s; process group killed",
                inv.timeout.as_secs_f64()
            ));
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let t = WatchTimings::default();
        let secs: Vec<u64> = (0..12).map(|n| t.backoff(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        assert_eq!(t.backoff(u32::MAX).as_secs(), 300);
    }
}
