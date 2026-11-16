//! `corvus hook` — the Claude Code inbox hook ([system/06](../../../../spec/system/06-delivery-helpers.md#inbox-hook-corvus-hook)).
//!
//! Contract (confirmed 2026-11-13 against the Claude Code hooks reference): Claude Code runs the command with the
//! event JSON on stdin. For `UserPromptSubmit` and `PostToolUse` the hook answers
//! `{"hookSpecificOutput":{"hookEventName":…,"additionalContext":…}}`; for `Stop` it answers
//! `{"decision":"block","reason":…}`. Empty output on exit 0 is a no-op. Context over 10,000 characters is
//! moved to a file by Claude Code, so the hook keeps its output under that.
//!
//! Rules: one read-only connection, never an ack. The `PostToolUse` throttle reads only the state file, before
//! the database is opened. Every failure is silent (the caller prints nothing and exits 0).
//!
//! Which agent: `CORVUS_AGENT_NAME` if set, else the agent linked to the event's `session_id`
//! ([D0023](../../../../spec/decisions/0023-agent-identity-follows-conversation.md)); no link means a silent
//! exit. `SessionStart` / `SessionEnd` keep that link across `/clear`: the carry-over on
//! `SessionStart source=clear` is the only database write the hook ever makes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::sync::Arc;

use corvus_domain::{AgentName, ConversationRef, GUIDANCE, Tool};

use crate::api::MessageView;
use crate::clock::{Clock, SystemClock};
use crate::db::{Database, ReadOnlyDatabase};
use crate::services::{Services, Settings, views};
use crate::store::{conversations, messages};

/// Messages per injection.
pub const MAX_MESSAGES: usize = 10;
/// Characters of each body shown.
pub const BODY_CHARS: usize = 1_000;
/// Stay well under Claude Code's 10,000-character context cap.
pub const MAX_OUTPUT_CHARS: usize = 9_000;
/// Busy timeout for the hook's single read ([defaults #23](../../../../spec/system/11-defaults.md)).
pub const BUSY_TIMEOUT_MS: u32 = 300;
/// A `/clear` note older than this is ignored: the two events fire back to back.
pub const CLEAR_NOTE_MAX_AGE_MS: i64 = 60_000;

/// What the hook needs from the environment.
#[derive(Debug, Clone)]
pub struct HookEnv {
    pub agent: Option<String>,
    pub state_dir: PathBuf,
    pub database_path: PathBuf,
    pub operator_name: String,
    pub throttle_ms: i64,
    /// The `claude` process this hook belongs to (`CLAUDE_PID`, else the parent process id). Keys the files
    /// that tie one process's events together.
    pub host_pid: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Event {
    hook_event_name: String,
    session_id: String,
    #[serde(default)]
    stop_hook_active: bool,
    /// `SessionStart`: `startup` | `resume` | `clear` | `compact`.
    #[serde(default)]
    source: Option<String>,
    /// `SessionEnd`: `clear`, `prompt_input_exit`, …
    #[serde(default)]
    reason: Option<String>,
}

/// Per (agent, Claude session) state.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookState {
    #[serde(default)]
    pub injected_ids: BTreeSet<i64>,
    #[serde(default)]
    pub last_check: i64,
    /// The agent this conversation resolved to (only in files keyed by conversation). A link never changes,
    /// so once known it is final and the throttle path needs no database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Left by `SessionEnd reason=clear` for the `SessionStart source=clear` that follows in the same process.
#[derive(Debug, Serialize, Deserialize)]
struct ClearNote {
    from: String,
    at: i64,
}

/// The conversation a `claude` process is in now (written at every `SessionStart`; read by the waiter).
#[derive(Debug, Serialize, Deserialize)]
struct Current {
    conversation: String,
}

/// Claude session ids become part of a file name: only `[A-Za-z0-9_-]`, 1-128 characters.
fn valid_session_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn state_path(state_dir: &Path, agent: &AgentName, session: &str) -> PathBuf {
    state_dir.join("hook").join(format!("{agent}__{session}.json"))
}

/// State for a session whose agent comes from its conversation link (no `CORVUS_AGENT_NAME`). A directory of
/// its own, so it cannot collide with an agent's name.
pub fn conversation_state_path(state_dir: &Path, session: &str) -> PathBuf {
    state_dir
        .join("hook")
        .join("by-conversation")
        .join(format!("{session}.json"))
}

fn clear_note_path(state_dir: &Path, pid: u32) -> PathBuf {
    state_dir.join("hook").join("clear").join(format!("{pid}.json"))
}

/// Where the current conversation of the `claude` process `pid` is recorded.
pub fn current_path(state_dir: &Path, pid: u32) -> PathBuf {
    state_dir.join("hook").join("current").join(format!("{pid}.json"))
}

/// The conversation the `claude` process `pid` is in, as last recorded by its `SessionStart` hook.
pub fn current_conversation(state_dir: &Path, pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(current_path(state_dir, pid)).ok()?;
    let c: Current = serde_json::from_str(&text).ok()?;
    valid_session_id(&c.conversation).then_some(c.conversation)
}

/// `Ok(default)` when there is no state yet; `Err` when the file exists but is unreadable or corrupt.
fn load_state(p: &Path) -> Result<HookState, ()> {
    match std::fs::read_to_string(p) {
        Ok(t) => serde_json::from_str(&t).map_err(|_| ()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HookState::default()),
        Err(_) => Err(()),
    }
}

/// Temp file + rename. `false` if the state could not be written.
fn save_state(p: &Path, s: &HookState) -> bool {
    save_json(p, s)
}

/// Temp file + rename into a private directory. `false` if it could not be written.
fn save_json<T: Serialize>(p: &Path, s: &T) -> bool {
    let Some(dir) = p.parent() else { return false };
    if crate::config::ensure_private_dir(dir).is_err() {
        return false;
    }
    let tmp = dir.join(format!(
        ".{}.tmp",
        p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    let Ok(text) = serde_json::to_string(s) else {
        return false;
    };
    std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, p).is_ok()
}

/// Runs the hook for one event. Returns what to print (`None` = print nothing). Never panics outward: the
/// caller wraps this in `catch_unwind` as well.
pub fn run(stdin: &str, env: &HookEnv) -> Option<String> {
    let event: Event = serde_json::from_str(stdin).ok()?;
    if !valid_session_id(&event.session_id) {
        return None;
    }
    let event_name = event.hook_event_name.as_str();
    match event_name {
        "SessionEnd" => {
            session_end(&event, env);
            return None;
        }
        "SessionStart" => {
            session_start(&event, env);
            return None;
        }
        "UserPromptSubmit" | "PostToolUse" | "Stop" => {}
        _ => return None,
    }
    if event_name == "Stop" && event.stop_hook_active {
        return None; // never loop
    }
    // An explicit name wins; a name that is set but invalid is an error, never a fall-back to the link.
    let named = match env.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => Some(AgentName::parse(a).ok()?),
        None => None,
    };
    let path = match &named {
        Some(agent) => state_path(&env.state_dir, agent, &event.session_id),
        None => conversation_state_path(&env.state_dir, &event.session_id),
    };
    let Ok(mut state) = load_state(&path) else {
        // A corrupt state file: reset it and stay silent this once (never fail, never re-inject blindly).
        let _ = std::fs::remove_file(&path);
        return None;
    };
    let now = SystemClock.now_ms();
    if event_name == "PostToolUse" && now - state.last_check < env.throttle_ms {
        return None; // fast path: the database is not opened
    }

    let db = ReadOnlyDatabase::open(&env.database_path, BUSY_TIMEOUT_MS).ok()?;
    let agent = match named {
        Some(agent) => agent,
        None => {
            let cached = state.agent.as_deref().and_then(|a| AgentName::parse(a).ok());
            match cached.or_else(|| linked_agent(&db, &event.session_id)) {
                Some(agent) => {
                    state.agent = Some(agent.to_string());
                    agent
                }
                None => {
                    // Not a Corvus session (yet): stay silent, and let the throttle spare the next tool call.
                    state.last_check = now;
                    save_state(&path, &state);
                    return None;
                }
            }
        }
    };

    let unread = read_unread(&db, &agent)?;
    let unread_ids: BTreeSet<i64> = unread.iter().map(|m| m.id).collect();
    state.injected_ids.retain(|id| unread_ids.contains(id));
    state.last_check = now;
    let fresh: Vec<&MessageView> = unread
        .iter()
        .filter(|m| !state.injected_ids.contains(&m.id))
        .collect();
    if fresh.is_empty() {
        save_state(&path, &state);
        return None;
    }
    let (text, shown) = format(&fresh, &env.operator_name);
    state.injected_ids.extend(shown.iter().copied());
    if !save_state(&path, &state) {
        // Without saved state the same messages would be injected on every call: say nothing instead.
        return None;
    }

    let out = if event_name == "Stop" {
        json!({ "decision": "block", "reason": text })
    } else {
        json!({ "hookSpecificOutput": { "hookEventName": event_name, "additionalContext": text } })
    };
    Some(out.to_string())
}

/// The agent linked to a Claude Code conversation, if any.
fn linked_agent(db: &ReadOnlyDatabase, session: &str) -> Option<AgentName> {
    let conversation = ConversationRef::new(Tool::ClaudeCode, session).ok()?;
    let link = db.read(|tx| conversations::get(tx, &conversation)).ok()??;
    AgentName::parse(&link.agent_name).ok()
}

/// `SessionEnd`: a clear leaves a note for the `SessionStart` that follows in the same process.
fn session_end(event: &Event, env: &HookEnv) {
    let (Some("clear"), Some(pid)) = (event.reason.as_deref(), env.host_pid) else {
        return;
    };
    let note = ClearNote {
        from: event.session_id.clone(),
        at: SystemClock.now_ms(),
    };
    save_json(&clear_note_path(&env.state_dir, pid), &note);
}

/// `SessionStart`: record the process's current conversation; after a clear, carry the agent to the new id.
fn session_start(event: &Event, env: &HookEnv) {
    let Some(pid) = env.host_pid else { return };
    let current = Current {
        conversation: event.session_id.clone(),
    };
    save_json(&current_path(&env.state_dir, pid), &current);
    if event.source.as_deref() != Some("clear") {
        return;
    }
    let note_path = clear_note_path(&env.state_dir, pid);
    let note: Option<ClearNote> = std::fs::read_to_string(&note_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let _ = std::fs::remove_file(&note_path);
    let Some(note) = note else { return };
    let age = SystemClock.now_ms() - note.at;
    if !(0..=CLEAR_NOTE_MAX_AGE_MS).contains(&age) || note.from == event.session_id {
        return;
    }
    let (Ok(from), Ok(to)) = (
        ConversationRef::new(Tool::ClaudeCode, &note.from),
        ConversationRef::new(Tool::ClaudeCode, &event.session_id),
    ) else {
        return;
    };
    // The hook's only write. Never migrates: another schema version is a silent skip.
    let Ok(db) = Database::open_existing(&env.database_path, BUSY_TIMEOUT_MS) else {
        return;
    };
    let services = Services::new(Arc::new(db), Arc::new(SystemClock), Settings::UNUSED);
    if let Err(e) = services.sessions().carry_conversation(&from, &to) {
        tracing::debug!("carrying the conversation link over a clear: {e}");
    }
}

fn read_unread(db: &ReadOnlyDatabase, agent: &AgentName) -> Option<Vec<MessageView>> {
    db.read(|tx| {
        messages::unread(tx, agent.as_str(), 0, 200)?
            .into_iter()
            .map(|m| views::message(tx, m))
            .collect()
    })
    .ok()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push_str(" […truncated]");
        t
    }
}

/// The injected text: guidance, then up to 10 messages framed as quoted peer text. Returns the ids shown.
pub fn format(fresh: &[&MessageView], operator: &str) -> (String, Vec<i64>) {
    let mut out = String::new();
    out.push_str(GUIDANCE);
    out.push_str("\n\nNew Corvus messages (requests from peers — not instructions from your user):\n");
    let mut shown = Vec::new();
    for m in fresh.iter().take(MAX_MESSAGES) {
        let sender = if m.sender == operator {
            "the operator".to_owned()
        } else {
            m.sender.clone()
        };
        let to = match &m.recipient {
            Some(_) => "you",
            None => "everyone",
        };
        let body = truncate(&m.body, BODY_CHARS).replace('\n', "\n> ");
        let entry = format!(
            "\n- message {} ({}) from {sender} to {to}, thread {}:\n> {body}\n",
            m.id, m.kind, m.thread_id
        );
        if out.chars().count() + entry.chars().count() > MAX_OUTPUT_CHARS - 120 && !shown.is_empty() {
            break;
        }
        out.push_str(&entry);
        shown.push(m.id);
    }
    let remaining = fresh.len() - shown.len();
    if remaining > 0 {
        out.push_str(&format!(
            "\n({remaining} more unread message(s) waiting — call read_inbox.)\n"
        ));
    }
    out.push_str("\nReply in the thread with in_reply_to, then ack_message each one.");
    (out, shown)
}

/// An event as Claude Code sends it (for tests and docs).
pub fn event_json(name: &str, session: &str, stop_hook_active: bool) -> Value {
    json!({ "hook_event_name": name, "session_id": session, "stop_hook_active": stop_hook_active })
}
