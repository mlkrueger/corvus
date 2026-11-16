//! Sessions and presence (brief §4.1, system/03 "Sessions and presence").

use corvus_domain::{
    AgentName, ConversationRef, CorvusError, EndReason, EventKind, Result, SessionEndReason, TaskStatus,
    Tool, generated_name, suffixed_name,
};

use crate::api::{
    ConversationInput, ConversationView, EndStaleResult, ListSessions, RegisterSession, SessionView,
};
use crate::db::WriteTx;
use crate::ids::{new_session_id, random_u64};
use crate::services::{Services, agent, idempotency_key, label, session_id, touch, views};
use crate::store::events::{self, NewEvent};
use crate::store::{agents, claims, conversations, sessions, tasks};

/// Generated-name attempts before falling back to a numeric suffix ([defaults #17]).
const NAME_ATTEMPTS: u32 = 20;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_DIRECTORY_CHARS: usize = 1024;

/// Validates a conversation reference as received from a transport.
fn conversation_ref(input: &ConversationInput) -> Result<ConversationRef> {
    ConversationRef::new(input.tool.parse::<Tool>()?, &input.id)
}

pub(super) fn conversation_view(row: conversations::ConversationRow) -> ConversationView {
    ConversationView {
        tool: row.tool,
        conversation_id: row.conversation_id,
        agent_name: row.agent_name,
        directory: row.directory,
        carried_from: row.carried_from,
        created_at: crate::clock::Timestamp(row.created_at),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Sessions<'a> {
    pub(super) s: &'a Services,
}

impl Sessions<'_> {
    /// Creates a session (and the agent, if new). Without a name, generates and atomically reserves one.
    /// Idempotent on `(agent_name, idempotency_key)`; a replay returns the original session.
    ///
    /// With a `conversation`: a linked conversation registers as its linked agent (a different requested name
    /// is ignored); an unlinked one is linked to the agent registered here, in the same transaction.
    pub fn register(&self, req: &RegisterSession) -> Result<SessionView> {
        let conversation = req.conversation.as_ref().map(conversation_ref).transpose()?;
        let directory = req
            .conversation
            .as_ref()
            .and_then(|c| c.directory.as_deref())
            .map(str::trim)
            .filter(|d| !d.is_empty() && d.chars().count() <= MAX_DIRECTORY_CHARS && !d.contains('\0'))
            .map(str::to_owned);
        let name = req.agent_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let name = name.map(|n| agent("agent_name", n)).transpose()?;
        let client = label("client_name", &req.client_name, 200)?;
        let metadata = match &req.metadata {
            None | Some(serde_json::Value::Null) => "{}".to_owned(),
            Some(v @ serde_json::Value::Object(_)) => {
                serde_json::to_string(v).map_err(|e| CorvusError::Validation(format!("metadata: {e}")))?
            }
            Some(_) => return Err(CorvusError::Validation("metadata: must be a JSON object".into())),
        };
        if metadata.len() > MAX_METADATA_BYTES {
            return Err(CorvusError::Validation(format!(
                "metadata: at most {MAX_METADATA_BYTES} bytes"
            )));
        }
        let key = idempotency_key(&req.idempotency_key)?;
        let now = self.s.now();
        let settings = self.s.settings();

        let row = self.s.db().write(|tx| {
            let linked = match &conversation {
                Some(c) => conversations::get(tx, c)?
                    .map(|l| agent("agent_name", &l.agent_name))
                    .transpose()?,
                None => None,
            };
            let is_linked = linked.is_some();
            let name = linked.or_else(|| name.clone());
            if let Some(k) = &key {
                let replay = match &name {
                    Some(n) => sessions::by_idempotency_key(tx, n.as_str(), k)?,
                    None => sessions::generated_by_idempotency_key(tx, k)?,
                };
                if let Some(row) = replay {
                    return Ok(row);
                }
            }
            let (agent_name, generated) = match &name {
                Some(n) => {
                    agents::ensure(tx, n, now)?;
                    (n.clone(), false)
                }
                None => (reserve_generated_name(tx, now)?, true),
            };
            let id = new_session_id().to_string();
            let inserted = sessions::insert(
                tx,
                &sessions::NewSession {
                    id: &id,
                    agent_name: agent_name.as_str(),
                    client_name: &client,
                    metadata: &metadata,
                    name_generated: generated,
                    idempotency_key: key.as_deref(),
                },
                now,
            )?;
            if !inserted {
                // Lost an idempotency race inside the lock: impossible, since we checked above in the same
                // transaction, but stay correct regardless.
                let k = key.as_deref().unwrap_or_default();
                return sessions::by_idempotency_key(tx, agent_name.as_str(), k)?
                    .ok_or_else(|| CorvusError::Internal("idempotent session vanished".into()));
            }
            events::insert(
                tx,
                EventKind::SessionRegistered,
                &NewEvent {
                    summary: format!("{agent_name} registered ({client})"),
                    agent_name: Some(agent_name.as_str()),
                    session_id: Some(&id),
                    ..NewEvent::default()
                },
                now,
            )?;
            if let Some(c) = conversation.as_ref().filter(|_| !is_linked) {
                conversations::insert(tx, c, agent_name.as_str(), directory.as_deref(), None, now)?;
                events::insert(
                    tx,
                    EventKind::ConversationLinked,
                    &NewEvent {
                        summary: format!("{agent_name} is {} conversation {}", c.tool(), c.id()),
                        agent_name: Some(agent_name.as_str()),
                        session_id: Some(&id),
                        ..NewEvent::default()
                    },
                    now,
                )?;
            }
            sessions::get(tx, &id)?.ok_or_else(|| CorvusError::Internal("session vanished".into()))
        })?;
        views::session(row, now, &settings)
    }

    /// The agent a tool conversation is linked to, if any.
    pub fn agent_for_conversation(&self, conversation: &ConversationRef) -> Result<Option<AgentName>> {
        self.s.db().read(|tx| {
            conversations::get(tx, conversation)?
                .map(|l| agent("agent_name", &l.agent_name))
                .transpose()
        })
    }

    /// An agent's conversation links, newest first.
    pub fn conversations_for_agent(&self, agent_name: &str) -> Result<Vec<ConversationView>> {
        let name = agent("agent_name", agent_name)?;
        self.s.db().read(|tx| {
            Ok(conversations::for_agent(tx, name.as_str())?
                .into_iter()
                .map(conversation_view)
                .collect())
        })
    }

    /// Every agent's newest conversation link (the one a wake is sent to).
    pub fn newest_conversations(&self) -> Result<Vec<ConversationView>> {
        self.s.db().read(|tx| {
            Ok(conversations::newest_per_agent(tx)?
                .into_iter()
                .map(conversation_view)
                .collect())
        })
    }

    /// The tool issued a new id for the same session (Claude Code `/clear`): links `to` to the agent `from`
    /// is linked to. Returns that agent, or `None` when `from` is not linked or `to` already is (no change).
    pub fn carry_conversation(
        &self,
        from: &ConversationRef,
        to: &ConversationRef,
    ) -> Result<Option<AgentName>> {
        if from.tool() != to.tool() {
            return Err(CorvusError::Validation(
                "conversation: a link is carried within one tool".into(),
            ));
        }
        let now = self.s.now();
        self.s.db().write(|tx| {
            let Some(link) = conversations::get(tx, from)? else {
                return Ok(None);
            };
            if !conversations::insert(
                tx,
                to,
                &link.agent_name,
                link.directory.as_deref(),
                Some(from.id()),
                now,
            )? {
                return Ok(None);
            }
            events::insert(
                tx,
                EventKind::ConversationCarried,
                &NewEvent {
                    summary: format!(
                        "{} carried to {} conversation {} (from {})",
                        link.agent_name,
                        to.tool(),
                        to.id(),
                        from.id()
                    ),
                    agent_name: Some(&link.agent_name),
                    ..NewEvent::default()
                },
                now,
            )?;
            agent("agent_name", &link.agent_name).map(Some)
        })
    }

    /// Bumps `last_seen_at`. Unknown or ended → `SessionNotActive`. Not audited (liveness, not a state change).
    pub fn heartbeat(&self, id: &str) -> Result<SessionView> {
        let id = session_id(id)?;
        let now = self.s.now();
        let row = self.s.db().write(|tx| {
            touch(tx, &id, now)?;
            sessions::get(tx, &id.to_string())?.ok_or(CorvusError::SessionNotActive)
        })?;
        views::session(row, now, &self.s.settings())
    }

    /// Ends the session, releasing its claim (the task reopens). Idempotent: ending an ended session returns
    /// it unchanged. Unknown → `SessionNotActive`.
    pub fn end(&self, id: &str) -> Result<SessionView> {
        let id = session_id(id)?.to_string();
        let now = self.s.now();
        let row = self.s.db().write(|tx| {
            let Some(row) = sessions::get(tx, &id)? else {
                return Err(CorvusError::SessionNotActive);
            };
            if row.ended_at.is_some() {
                return Ok(row);
            }
            end_one(tx, &id, &row.agent_name, SessionEndReason::Ended, now)?;
            sessions::get(tx, &id)?.ok_or(CorvusError::SessionNotActive)
        })?;
        views::session(row, now, &self.s.settings())
    }

    pub fn get(&self, id: &str) -> Result<SessionView> {
        let id = session_id(id)?.to_string();
        let now = self.s.now();
        let row = self
            .s
            .db()
            .read(|tx| sessions::get(tx, &id))?
            .ok_or_else(|| CorvusError::NotFound(format!("session {id} not found")))?;
        views::session(row, now, &self.s.settings())
    }

    /// Sessions (live only unless `include_ended`), newest first, with derived presence.
    pub fn list(&self, req: &ListSessions) -> Result<Vec<SessionView>> {
        let agent_filter = req.agent.as_deref().map(|a| agent("agent", a)).transpose()?;
        let now = self.s.now();
        let settings = self.s.settings();
        let rows = self.s.db().read(|tx| {
            sessions::list(
                tx,
                req.include_ended,
                agent_filter.as_ref().map(AgentName::as_str),
            )
        })?;
        rows.into_iter()
            .map(|r| views::session(r, now, &settings))
            .collect()
    }

    /// Auto-end: one transaction ends every live session silent longer than the threshold, releasing claims.
    /// A heartbeat that commits first moves `last_seen_at`, so it is not selected ("a concurrent heartbeat
    /// wins"). Threshold 0 disables.
    pub fn end_stale(&self) -> Result<EndStaleResult> {
        let threshold = self.s.settings().auto_end_ms;
        if threshold <= 0 {
            return Ok(EndStaleResult { ended: Vec::new() });
        }
        let now = self.s.now();
        let ended = self.s.db().write(|tx| {
            let stale = sessions::stale(tx, now - threshold)?;
            for (id, agent_name) in &stale {
                end_one(tx, id, agent_name, SessionEndReason::AutoEnded, now)?;
            }
            Ok(stale.into_iter().map(|(id, _)| id).collect())
        })?;
        Ok(EndStaleResult { ended })
    }
}

/// Ends a live session inside the caller's transaction: releases its claim, reopens the task, one event.
fn end_one(tx: &WriteTx<'_>, id: &str, agent_name: &str, reason: SessionEndReason, now: i64) -> Result<()> {
    let claim = claims::active_for_session(tx, id)?;
    if let Some(c) = &claim {
        claims::end(tx, &c.id, EndReason::Released, now)?;
        tasks::set_status(tx, &c.task_id, TaskStatus::Open, now)?;
    }
    sessions::end(tx, id, reason, now)?;
    let (kind, verb) = match reason {
        SessionEndReason::Ended => (EventKind::SessionEnded, "ended"),
        SessionEndReason::AutoEnded => (EventKind::SessionAutoEnded, "auto-ended"),
    };
    let summary = match &claim {
        Some(_) => format!("{agent_name} session {verb}; its task was reopened"),
        None => format!("{agent_name} session {verb}"),
    };
    events::insert(
        tx,
        kind,
        &NewEvent {
            summary,
            agent_name: Some(agent_name),
            session_id: Some(id),
            task_id: claim.as_ref().map(|c| c.task_id.as_str()),
            ..NewEvent::default()
        },
        now,
    )?;
    Ok(())
}

/// Picks `<adjective>-<bird>` at random and reserves it with an insert; 20 attempts, then `-2`, `-3`, …
fn reserve_generated_name(tx: &WriteTx<'_>, now: i64) -> Result<AgentName> {
    let pick = || {
        let r = random_u64();
        generated_name((r & 0xffff_ffff) as usize, (r >> 32) as usize)
    };
    for _ in 0..NAME_ATTEMPTS {
        let n = pick();
        if agents::ensure(tx, &n, now)? {
            return Ok(n);
        }
    }
    let base = pick();
    for i in 2..=100_000u32 {
        let n = suffixed_name(&base, i);
        if agents::ensure(tx, &n, now)? {
            return Ok(n);
        }
    }
    Err(CorvusError::Internal(
        "could not generate a free agent name".into(),
    ))
}
