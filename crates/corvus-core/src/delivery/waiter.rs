//! `corvus hook wait` — wakes an idle Claude Code session when mail arrives
//! ([system/06](../../../../spec/system/06-delivery-helpers.md#waiter-corvus-hook-wait),
//! [D0024](../../../../spec/decisions/0024-wake-through-the-tools-own-channel.md)).
//!
//! Claude Code runs a hook marked `asyncRewake` in the background and starts a turn with its stderr when it
//! exits with code 2. This is that hook: it waits on the inbox (a `PRAGMA data_version` poll on a read-only
//! connection, no model tokens) and returns the text to wake with — or `None`, which the caller turns into a
//! silent exit 0. It never writes to the database, never acks, and never fails outward.
//!
//! One waiter per `claude` process (a file lock). It follows that process's current conversation, so it
//! survives `/clear` and an in-session resume, and it shares the inbox hook's shown-set, so a message wakes a
//! session once.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use corvus_domain::{AgentName, ConversationRef, Tool};

use crate::api::MessageView;
use crate::config::ensure_private_dir;
use crate::db::{HintReader, ReadOnlyDatabase};
use crate::delivery::hook::{
    BUSY_TIMEOUT_MS, HookEnv, conversation_state_path, current_conversation, format, load_state, save_state,
    state_path, valid_session_id,
};
use crate::services::views;
use crate::store::{conversations, messages};

/// How often the waiter looks at `data_version`, its parent and its current conversation.
pub const TICK: Duration = Duration::from_millis(250);
/// Look at the inbox at least this often even if `data_version` did not move (a lost hint only delays).
pub const RECHECK: Duration = Duration::from_secs(10);
/// How often a session with no linked agent yet is looked up again.
pub const UNLINKED_RECHECK: Duration = Duration::from_secs(5);

/// What the waiter needs beyond the hook's environment.
#[derive(Debug, Clone)]
pub struct WaitEnv {
    pub hook: HookEnv,
    /// The session is not interactive (`CLAUDE_CODE_ENTRYPOINT` set and not `cli`). A waiter would keep
    /// `claude -p` from ever exiting, so it does not run there.
    pub headless: bool,
}

#[derive(serde::Deserialize)]
struct Event {
    session_id: String,
}

/// `$STATE/hook/wait/<pid>.lock`.
pub fn lock_path(state_dir: &Path, pid: u32) -> PathBuf {
    state_dir.join("hook").join("wait").join(format!("{pid}.lock"))
}

/// The exclusive per-process waiter lock, held for the waiter's lifetime (the OS releases it on exit).
/// `None` when another waiter holds it or it cannot be taken.
fn acquire(state_dir: &Path, pid: u32) -> Option<File> {
    let path = lock_path(state_dir, pid);
    ensure_private_dir(path.parent()?).ok()?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .ok()?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock | TryLockError::Error(_)) => return None,
    }
    if file.set_len(0).is_ok() {
        let _ = write!(file, "{}", std::process::id());
    }
    Some(file)
}

/// Whether a waiter currently holds the lock for the `claude` process `pid`.
pub fn is_waiting(state_dir: &Path, pid: u32) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path(state_dir, pid))
    else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// A message that wakes: addressed to the agent, or a broadcast the operator marked `wake`.
fn wakes(m: &MessageView, agent: &AgentName) -> bool {
    match &m.recipient {
        Some(to) => to == agent.as_str(),
        None => m.wake,
    }
}

/// Waits until there is mail to wake the session with. Returns the text for stderr (the caller exits 2), or
/// `None` for a silent exit 0: headless, another waiter running, the parent gone, `stop` set, or any error.
pub fn wait(stdin: &str, env: &WaitEnv, stop: &AtomicBool) -> Option<String> {
    if env.headless {
        return None;
    }
    let event: Event = serde_json::from_str(stdin).ok()?;
    if !valid_session_id(&event.session_id) {
        return None;
    }
    let pid = env.hook.host_pid?;
    let state_dir = &env.hook.state_dir;
    // An explicit name wins; one that is set but invalid is an error, never a fall-back to the link.
    let named = match env.hook.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => Some(AgentName::parse(a).ok()?),
        None => None,
    };
    // Fully migrated database or nothing: the waiter never migrates and never waits for one to appear.
    drop(ReadOnlyDatabase::open(&env.hook.database_path, BUSY_TIMEOUT_MS).ok()?);
    let _lock = acquire(state_dir, pid)?;
    let mut reader = HintReader::open(&env.hook.database_path).ok()?;
    let parent = std::os::unix::process::parent_id();

    let mut conversation = String::new();
    let mut agent: Option<AgentName> = None;
    let mut last_version: Option<i64> = None;
    let mut last_check = Instant::now();
    let mut last_lookup: Option<Instant> = None;
    loop {
        if stop.load(Ordering::Relaxed) || std::os::unix::process::parent_id() != parent {
            return None; // told to stop, or the session was killed and orphaned us
        }
        // Follow the process's current conversation (a clear or an in-session resume changes it).
        let now_in = current_conversation(state_dir, pid).unwrap_or_else(|| event.session_id.clone());
        let moved = now_in != conversation;
        if moved {
            conversation = now_in;
            agent = named.clone();
            last_lookup = None;
        }
        if agent.is_none() && last_lookup.is_none_or(|t| t.elapsed() >= UNLINKED_RECHECK) {
            last_lookup = Some(Instant::now());
            agent = linked_agent(&mut reader, &conversation);
        }
        let version = reader.data_version().ok()?;
        let due = moved || last_version != Some(version) || last_check.elapsed() >= RECHECK;
        if let (true, Some(agent)) = (due, &agent) {
            last_version = Some(version);
            last_check = Instant::now();
            if let Some(text) = check(&mut reader, env, agent, &conversation, named.is_some()) {
                return Some(text);
            }
        }
        std::thread::sleep(TICK);
    }
}

fn linked_agent(reader: &mut HintReader, session: &str) -> Option<AgentName> {
    let conversation = ConversationRef::new(Tool::ClaudeCode, session).ok()?;
    let link = reader.read(|tx| conversations::get(tx, &conversation)).ok()??;
    AgentName::parse(&link.agent_name).ok()
}

/// One look at the inbox. `Some(text)` when something new should wake the session; the shown-set is saved
/// first, and if it cannot be saved nothing is returned (never a wake loop).
fn check(
    reader: &mut HintReader,
    env: &WaitEnv,
    agent: &AgentName,
    conversation: &str,
    named: bool,
) -> Option<String> {
    let unread: Vec<MessageView> = reader
        .read(|tx| {
            messages::unread(tx, agent.as_str(), 0, 200)?
                .into_iter()
                .map(|m| views::message(tx, m))
                .collect()
        })
        .ok()?;
    let path = if named {
        state_path(&env.hook.state_dir, agent, conversation)
    } else {
        conversation_state_path(&env.hook.state_dir, conversation)
    };
    let mut state = load_state(&path).ok()?;
    let unread_ids: BTreeSet<i64> = unread.iter().map(|m| m.id).collect();
    state.injected_ids.retain(|id| unread_ids.contains(id));
    let fresh: Vec<&MessageView> = unread
        .iter()
        .filter(|m| !state.injected_ids.contains(&m.id))
        .collect();
    if !fresh.iter().any(|m| wakes(m, agent)) {
        return None;
    }
    // Waking anyway: show everything new, ordinary broadcasts included, as the inbox hook would.
    let (text, shown) = format(&fresh, &env.hook.operator_name);
    state.injected_ids.extend(shown.iter().copied());
    if !named {
        state.agent = Some(agent.to_string());
    }
    save_state(&path, &state).then_some(text)
}
