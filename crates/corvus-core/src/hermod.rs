//! Hermod: cross-process wake-ups ([system/03](../../../spec/system/03-services.md#hermod-wake-ups), D0016).
//!
//! Triggers append topics (`inbox:<agent>`, `inbox:*`, `tasks`) to `hints` in the same transaction as each
//! change. While this process has subscribers, one watcher task reads `PRAGMA data_version` on a dedicated
//! read connection every 250 ms; it changes whenever any other connection, in any process, commits. On a
//! change the watcher reads hints past its cursor and wakes subscribers whose topics match. If hints were
//! pruned past the cursor it wakes everyone. With no subscribers the watcher stops: no waiter, no polling.
//!
//! Hints only say "go look". Consumers subscribe first, then read, then wait, and re-check periodically, so a
//! coalesced, duplicated or missing hint changes latency, never results.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use corvus_domain::{CorvusError, Result};

use crate::api::{MessageView, ReadInbox, WaitForInbox};
use crate::services::Services;
use tokio::sync::Notify;

use crate::db::HintReader;
use crate::store::hints;

/// Detection interval ([defaults #12](../../../spec/system/11-defaults.md)).
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Waiters re-check at least this often, so a lost hint costs latency, never a result.
pub const RECHECK_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HermodConfig {
    pub poll: Duration,
    pub recheck: Duration,
    /// Tests turn detection off to prove waiters still return through the periodic re-check.
    pub detect: bool,
}

impl Default for HermodConfig {
    fn default() -> Self {
        Self {
            poll: POLL_INTERVAL,
            recheck: RECHECK_INTERVAL,
            detect: true,
        }
    }
}

/// One per process. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Hermod {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    config: HermodConfig,
    state: Mutex<State>,
    /// How many times the watcher has polled (tests assert idleness with it).
    polls: AtomicU64,
}

#[derive(Debug, Default)]
struct State {
    next_id: u64,
    subs: HashMap<u64, Sub>,
    watcher_running: bool,
}

#[derive(Debug)]
struct Sub {
    topics: Vec<String>,
    notify: Arc<Notify>,
}

/// A live subscription. Dropping it unsubscribes.
#[derive(Debug)]
pub struct Subscription {
    id: u64,
    notify: Arc<Notify>,
    inner: Arc<Inner>,
}

impl Subscription {
    /// Resolves after a matching hint arrives (a hint that arrived since the last call counts).
    pub async fn notified(&self) {
        self.notify.notified().await;
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        lock(&self.inner.state).subs.remove(&self.id);
    }
}

impl Hermod {
    pub fn new(db_path: &Path) -> Self {
        Self::with_config(db_path, HermodConfig::default())
    }

    pub fn with_config(db_path: &Path, config: HermodConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                path: db_path.to_owned(),
                config,
                state: Mutex::new(State::default()),
                polls: AtomicU64::new(0),
            }),
        }
    }

    pub fn config(&self) -> HermodConfig {
        self.inner.config
    }

    /// Subscribes to `topics`. Call this BEFORE reading state. Starts the watcher if it is not running; the
    /// watcher's cursor is captured here, so any hint committed after this call wakes the subscriber.
    pub async fn subscribe(&self, topics: &[String]) -> Result<Subscription> {
        let notify = Arc::new(Notify::new());
        let start_watcher = {
            let mut st = lock(&self.inner.state);
            let id = st.next_id;
            st.next_id += 1;
            st.subs.insert(
                id,
                Sub {
                    topics: topics.to_vec(),
                    notify: notify.clone(),
                },
            );
            let start = !st.watcher_running;
            st.watcher_running = true;
            (id, start)
        };
        let (id, start) = start_watcher;
        let sub = Subscription {
            id,
            notify,
            inner: self.inner.clone(),
        };
        if start {
            let path = self.inner.path.clone();
            let opened = tokio::task::spawn_blocking(move || -> Result<(HintReader, i64, i64)> {
                let mut reader = HintReader::open(&path)?;
                // Baseline data_version first, then the cursor: a commit in between changes data_version, so
                // the first poll catches it.
                let dv = reader.data_version()?;
                let cursor = reader.read(|tx| hints::max_id(tx))?;
                Ok((reader, dv, cursor))
            })
            .await
            .map_err(|e| CorvusError::Internal(format!("hermod: {e}")));
            match opened {
                Ok(Ok((reader, dv, cursor))) => {
                    tokio::spawn(watch(self.inner.clone(), reader, dv, cursor));
                }
                Ok(Err(e)) | Err(e) => {
                    lock(&self.inner.state).watcher_running = false;
                    return Err(e);
                }
            }
        }
        Ok(sub)
    }

    /// Number of watcher polls so far.
    pub fn polls(&self) -> u64 {
        self.inner.polls.load(Ordering::Relaxed)
    }

    pub fn watcher_running(&self) -> bool {
        lock(&self.inner.state).watcher_running
    }
}

/// One watcher check: the new `data_version`, the hints past the cursor, and whether hints were pruned past it.
type Check = Option<(i64, Vec<(i64, String)>, bool)>;

/// The watcher loop. Runs while there are subscribers; never dies on an error (logs and continues).
async fn watch(inner: Arc<Inner>, reader: HintReader, mut dv: i64, mut cursor: i64) {
    let reader = Arc::new(Mutex::new(reader));
    loop {
        tokio::time::sleep(inner.config.poll).await;
        {
            let mut st = lock(&inner.state);
            if st.subs.is_empty() {
                st.watcher_running = false;
                return;
            }
        }
        inner.polls.fetch_add(1, Ordering::Relaxed);
        if !inner.config.detect {
            continue;
        }
        let r = reader.clone();
        let step = tokio::task::spawn_blocking(move || -> Result<Check> {
            let mut reader = lock(&r);
            let now_dv = reader.data_version()?;
            if now_dv == dv {
                return Ok(None);
            }
            reader.read(|tx| {
                let rows = hints::after(tx, cursor)?;
                // Ids are AUTOINCREMENT and never reused, so a gap right after the cursor means hints were
                // pruned before this process saw them: one of them may have been ours.
                let pruned_past = rows.first().is_some_and(|(id, _)| *id > cursor + 1);
                Ok(Some((now_dv, rows, pruned_past)))
            })
        })
        .await;
        match step {
            Ok(Ok(Some((new_dv, rows, pruned_past)))) => {
                dv = new_dv;
                if let Some((last, _)) = rows.last() {
                    cursor = *last;
                }
                let st = lock(&inner.state);
                for sub in st.subs.values() {
                    if pruned_past
                        || rows
                            .iter()
                            .any(|(_, topic)| sub.topics.iter().any(|t| t == topic))
                    {
                        sub.notify.notify_one();
                    }
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "hermod: hint check failed; continuing"),
            Err(e) => tracing::warn!(error = %e, "hermod: hint check panicked; continuing"),
        }
    }
}

/// The longest a single wait may block ([defaults #11](../../../spec/system/11-defaults.md)).
pub const MAX_WAIT: Duration = Duration::from_secs(120);
const DEFAULT_WAIT: Duration = Duration::from_secs(60);

/// `timeout_seconds` clamped to 0..=120 (default 60).
pub fn clamp_timeout(seconds: Option<u64>) -> Duration {
    seconds.map_or(DEFAULT_WAIT, |s| Duration::from_secs(s).min(MAX_WAIT))
}

/// The canonical consumer (P5): subscribe FIRST, count as alive, then read; if nothing is unread, wait for a
/// matching hint or the periodic re-check, and read again. Returns as soon as unread messages exist, or empty
/// at the deadline. Never acks.
pub async fn wait_for_inbox(
    services: &Services,
    hermod: &Hermod,
    req: &WaitForInbox,
) -> Result<Vec<MessageView>> {
    let agent = crate::services::agent("agent", &req.agent)?;
    let timeout = clamp_timeout(req.timeout_seconds);
    let deadline = tokio::time::Instant::now() + timeout;
    let topics = [format!("inbox:{agent}"), "inbox:*".to_owned()];

    let sub = hermod.subscribe(&topics).await?; // 1. subscribe first
    if let Some(sid) = &req.session_id {
        // Waiting counts as alive.
        let (s, sid) = (services.clone(), sid.clone());
        blocking(move || s.sessions().heartbeat(&sid).map(|_| ())).await?;
    }
    let read = ReadInbox {
        agent: agent.to_string(),
        limit: req.limit,
        after_id: None,
    };
    loop {
        let (s, r) = (services.clone(), read.clone());
        let unread = blocking(move || s.muninn().read_inbox(&r)).await?; // 2. then read
        if !unread.is_empty() {
            return Ok(unread);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(Vec::new());
        }
        let nap = (deadline - now).min(hermod.config().recheck);
        tokio::select! { // 3. wait for a hint or the re-check
            () = sub.notified() => {}
            () = tokio::time::sleep(nap) => {}
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CorvusError::Internal(format!("blocking task failed: {e}")))?
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
