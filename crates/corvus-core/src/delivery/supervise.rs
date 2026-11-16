//! The background leader's wake step
//! ([system/06](../../../../spec/system/06-delivery-helpers.md#bound-from-the-conversation), defaults row 64).
//!
//! An agent with no explicit binding whose newest conversation is for a tool that has an adapter of the same
//! name is woken here, so a running desktop app (or `corvus serve`) is all the owner needs. Each such agent
//! gets the ordinary [`watch_loop`] in adapter mode: same per-agent lock, delivered set and back-off as
//! `corvus watch`, so the two never wake one agent twice. The set of agents is reconciled on a timer; a failed
//! reconcile keeps what is running.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvus_domain::{Adapter, AgentName, CorvusError, Result};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::delivery::watch::{Wake, WatchTimings, watch_loop};
use crate::delivery::{adapters, bindings};
use crate::facade::Corvus;

/// How often the set of supervised agents is re-read.
pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub struct SuperviseTimings {
    pub reconcile: Duration,
    pub watch: WatchTimings,
}

impl Default for SuperviseTimings {
    fn default() -> Self {
        Self {
            reconcile: RECONCILE_INTERVAL,
            watch: WatchTimings::default(),
        }
    }
}

/// What the leader needs to supervise wakes.
#[derive(Debug, Clone)]
pub struct WakeSupervision {
    pub corvus: Corvus,
    pub timings: SuperviseTimings,
}

impl WakeSupervision {
    pub fn new(corvus: Corvus) -> Self {
        Self {
            corvus,
            timings: SuperviseTimings::default(),
        }
    }
}

/// One agent to supervise: its tool's adapter and the conversation to wake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub adapter: Adapter,
    pub conversation: String,
}

/// The agents bound from a conversation: no entry in `bindings.json`, and the newest conversation is for a
/// tool with a loadable adapter of the same name whose pattern accepts the conversation id.
pub fn targets(corvus: &Corvus, state_dir: &Path) -> Result<BTreeMap<AgentName, Target>> {
    let explicit = bindings::list(state_dir)?;
    let mut loaded: HashMap<String, Option<Adapter>> = HashMap::new();
    let mut out = BTreeMap::new();
    for link in corvus.services().sessions().newest_conversations()? {
        if explicit.contains_key(&link.agent_name) {
            continue;
        }
        let adapter = loaded.entry(link.tool.clone()).or_insert_with(|| {
            match adapters::load(state_dir, &link.tool) {
                Ok(l) => Some(l.adapter),
                // A tool with no adapter (Claude Code) wakes itself.
                Err(CorvusError::NotFound(_)) => None,
                Err(e) => {
                    tracing::warn!(tool = %link.tool, error = %e, "adapter cannot be loaded; its agents are not woken");
                    None
                }
            }
        });
        let Some(adapter) = adapter else { continue };
        if let Err(e) = adapter.validate_binding(Some(&link.conversation_id)) {
            tracing::debug!(agent = %link.agent_name, error = %e, "conversation id does not fit the adapter");
            continue;
        }
        let Ok(agent) = AgentName::parse(&link.agent_name) else {
            continue;
        };
        out.insert(
            agent,
            Target {
                adapter: adapter.clone(),
                conversation: link.conversation_id,
            },
        );
    }
    Ok(out)
}

struct Running {
    target: Target,
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Runs until `shutdown` turns true or its sender is dropped, then stops every watcher it started and waits
/// for them (a running adapter command is killed).
pub async fn supervise(
    corvus: Corvus,
    state_dir: PathBuf,
    timings: SuperviseTimings,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut running: HashMap<AgentName, Running> = HashMap::new();
    loop {
        if *shutdown.borrow() {
            break;
        }
        let (c, dir) = (corvus.clone(), state_dir.clone());
        match tokio::task::spawn_blocking(move || targets(&c, &dir)).await {
            Ok(Ok(want)) => reconcile(&corvus, &state_dir, timings.watch, &mut running, want),
            Ok(Err(e)) => tracing::warn!(error = %e, "cannot list agents to wake; keeping the current set"),
            Err(e) => tracing::warn!(error = %e, "wake reconcile task failed; keeping the current set"),
        }
        tokio::select! {
            () = tokio::time::sleep(timings.reconcile) => {}
            changed = shutdown.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
    for (_, r) in running.drain() {
        let _ = r.stop.send(true);
        let _ = r.handle.await;
    }
}

fn reconcile(
    corvus: &Corvus,
    state_dir: &Path,
    timings: WatchTimings,
    running: &mut HashMap<AgentName, Running>,
    want: BTreeMap<AgentName, Target>,
) {
    // Stop watchers whose agent is no longer a target or whose conversation or adapter changed; forget the
    // ones that ended (another watcher holds the lock, or the state directory failed) so they are retried.
    running.retain(|agent, r| {
        let keep = !r.handle.is_finished() && want.get(agent) == Some(&r.target);
        if !keep {
            let _ = r.stop.send(true);
        }
        keep
    });
    for (agent, target) in want {
        if running.contains_key(&agent) {
            continue;
        }
        let (stop, rx) = watch::channel(false);
        let wake = Wake::adapter(target.adapter.clone(), Some(target.conversation.clone()), None);
        let (c, dir, a) = (corvus.clone(), state_dir.to_path_buf(), agent.clone());
        let handle = tokio::spawn(async move {
            match watch_loop(&c, &dir, &a, wake, timings, rx).await {
                Ok(()) => {}
                // `corvus watch`, or the previous watcher for a replaced conversation, still holds the lock.
                Err(CorvusError::Conflict(_)) => {
                    tracing::debug!(agent = %a, "another watcher holds the lock; will retry");
                }
                Err(e) => tracing::warn!(agent = %a, error = %e, "wake watcher ended; will retry"),
            }
        });
        running.insert(agent, Running { target, stop, handle });
    }
}
