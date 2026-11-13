//! Muninn: threads, messages, receipts, digests, decisions, bootstrap (brief §4.3, §4.4, system/03).

use corvus_domain::{
    AgentName, ArtifactRef, CorvusError, EventKind, MessageType, Result, Scope, direct_thread_key,
    validate_message_body, validate_summary, validate_thread_key, validate_title,
};

use crate::api::{
    AckMessage, AckResult, ArtifactInput, Bootstrap, CreateThread, DecisionView, DigestView, ListDecisions,
    ListMessages, MessageListItem, MessageView, PostMessage, ReadInbox, RecordDecision, SendDirect,
    ThreadView, WriteDigest,
};
use crate::db::{Reader, WriteTx};
use crate::ids::{new_artifact_id, new_decision_id, new_thread_id};
use crate::services::{Services, agent, idempotency_key, label, session_id, task_id, text, touch, views};
use crate::store::events::{self, NewEvent, clip};
use crate::store::{agents, artifacts, decisions, digests, messages, receipts, tasks, threads};

const DEFAULT_INBOX_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;
const MAX_ARTIFACTS: usize = 100;
const MAX_DECISION_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy)]
pub struct Muninn<'a> {
    pub(super) s: &'a Services,
}

/// Validated artifacts from a request.
pub(crate) fn artifact_inputs(inputs: &[ArtifactInput]) -> Result<Vec<(ArtifactRef, String)>> {
    if inputs.len() > MAX_ARTIFACTS {
        return Err(CorvusError::Validation(format!(
            "artifacts: at most {MAX_ARTIFACTS}"
        )));
    }
    inputs
        .iter()
        .map(|a| {
            let r = ArtifactRef::new(a.kind, a.path.clone(), a.commit.clone())?;
            let d = text("artifact description", &a.description, 2_000)?.unwrap_or_default();
            Ok((r, d))
        })
        .collect()
}

/// Touches `session` (if given) and checks it belongs to `actor`.
fn touch_as(
    tx: &WriteTx<'_>,
    session: Option<&corvus_domain::SessionId>,
    actor: &AgentName,
    now: i64,
) -> Result<()> {
    if let Some(sid) = session {
        let owner = touch(tx, sid, now)?;
        if &owner != actor {
            return Err(CorvusError::Conflict(format!(
                "session belongs to {owner}, not {actor}"
            )));
        }
    }
    Ok(())
}

fn limit(l: Option<u32>, default: u32) -> u32 {
    l.unwrap_or(default).clamp(1, MAX_LIMIT)
}

impl Muninn<'_> {
    /// Creates a thread. With a `key` it is idempotent (same key → same thread); `direct:` keys are reserved.
    /// With a `task_id` that already has a thread, that thread is returned.
    pub fn create_thread(&self, req: &CreateThread) -> Result<ThreadView> {
        let creator = agent("creator", &req.creator)?;
        validate_title(&req.title)?;
        let title = req.title.trim().to_owned();
        let key = req
            .key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_owned);
        if let Some(k) = &key {
            validate_thread_key(k)?;
            label("key", k, 200)?;
        }
        let task = req
            .task_id
            .as_deref()
            .map(|t| task_id("task_id", t))
            .transpose()?
            .map(|t| t.to_string());
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        self.s.db().write(|tx| {
            touch_as(tx, session.as_ref(), &creator, now)?;
            agents::ensure(tx, &creator, now)?;
            if let Some(t) = &task {
                if !tasks::exists(tx, t)? {
                    return Err(CorvusError::NotFound(format!("task {t} not found")));
                }
                if let Some(existing) = threads::for_task(tx, t)? {
                    return views::thread_by_id(tx, &existing);
                }
            }
            if let Some(k) = &key
                && let Some(existing) = threads::by_key(tx, k)?
            {
                return views::thread_by_id(tx, &existing);
            }
            let id = new_thread_id().to_string();
            threads::insert(
                tx,
                &id,
                &title,
                task.as_deref(),
                creator.as_str(),
                key.as_deref(),
                now,
            )?;
            events::insert(
                tx,
                EventKind::ThreadCreated,
                &NewEvent {
                    summary: format!("{creator} opened thread \"{}\"", clip(&title)),
                    agent_name: Some(creator.as_str()),
                    session_id: session.map(|s| s.to_string()).as_deref(),
                    task_id: task.as_deref(),
                    thread_id: Some(&id),
                    ..NewEvent::default()
                },
                now,
            )?;
            views::thread_by_id(tx, &id)
        })
    }

    pub fn get_thread(&self, id: &str) -> Result<ThreadView> {
        let id = thread_id(id)?;
        self.s.db().read(|tx| views::thread_by_id(tx, &id))
    }

    /// Messages in a thread, oldest first.
    pub fn thread_messages(
        &self,
        id: &str,
        limit_: Option<u32>,
        after_id: Option<i64>,
    ) -> Result<Vec<MessageView>> {
        let id = thread_id(id)?;
        let n = limit(limit_, MAX_LIMIT);
        self.s.db().read(|tx| {
            if threads::get(tx, &id)?.is_none() {
                return Err(CorvusError::NotFound(format!("thread {id} not found")));
            }
            messages::in_thread(tx, &id, after_id.unwrap_or(0), n)?
                .into_iter()
                .map(|m| views::message(tx, m))
                .collect()
        })
    }

    /// Posts a message. Idempotent on `(from, idempotency_key)`; the hint is written by trigger.
    pub fn post_message(&self, req: &PostMessage) -> Result<MessageView> {
        let from = agent("from", &req.from)?;
        let to = req
            .to
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| agent("to", t))
            .transpose()?;
        validate_message_body(&req.body)?;
        let thread = thread_id(&req.thread_id)?;
        let arts = artifact_inputs(&req.artifacts)?;
        let key = idempotency_key(&req.idempotency_key)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        self.s.db().write(|tx| {
            post_in_tx(
                tx,
                &thread,
                &from,
                to.as_ref(),
                req.kind,
                req.in_reply_to,
                &req.body,
                &arts,
                key.as_deref(),
                session.as_ref(),
                now,
            )
        })
    }

    /// One persistent thread per unordered agent pair (`direct:<min>:<max>`), then a message addressed to `to`.
    pub fn send_direct(&self, req: &SendDirect) -> Result<MessageView> {
        let from = agent("from", &req.from)?;
        let to = agent("to", &req.to)?;
        if from == to {
            return Err(CorvusError::Validation(
                "to: cannot send a direct message to yourself".into(),
            ));
        }
        validate_message_body(&req.body)?;
        let key = idempotency_key(&req.idempotency_key)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        let thread_key = direct_thread_key(&from, &to);
        self.s.db().write(|tx| {
            touch_as(tx, session.as_ref(), &from, now)?;
            agents::ensure(tx, &from, now)?;
            if !agents::exists(tx, &to)? {
                return Err(CorvusError::NotFound(format!("agent {to} not found")));
            }
            let thread = match threads::by_key(tx, &thread_key)? {
                Some(t) => t,
                None => {
                    let id = new_thread_id().to_string();
                    let (a, b) = if from.as_str() < to.as_str() {
                        (&from, &to)
                    } else {
                        (&to, &from)
                    };
                    let title = format!("Direct: {a} and {b}");
                    threads::insert(tx, &id, &title, None, from.as_str(), Some(&thread_key), now)?;
                    id
                }
            };
            // The session was already touched; pass None so it is not touched twice.
            post_in_tx(
                tx,
                &thread,
                &from,
                Some(&to),
                req.kind,
                None,
                &req.body,
                &[],
                key.as_deref(),
                None,
                now,
            )
        })
    }

    /// Unread messages for `agent`, oldest first. Never acks. `after_id` is only a paging hint.
    pub fn read_inbox(&self, req: &ReadInbox) -> Result<Vec<MessageView>> {
        let a = agent("agent", &req.agent)?;
        let n = limit(req.limit, DEFAULT_INBOX_LIMIT);
        self.s.db().read(|tx| {
            messages::unread(tx, a.as_str(), req.after_id.unwrap_or(0), n)?
                .into_iter()
                .map(|m| views::message(tx, m))
                .collect()
        })
    }

    /// Acknowledges a message for `agent`. Idempotent. Conflict for one's own message or one addressed to
    /// someone else.
    pub fn ack_message(&self, message_id: i64, req: &AckMessage) -> Result<AckResult> {
        let a = agent("agent", &req.agent)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        self.s.db().write(|tx| {
            touch_as(tx, session.as_ref(), &a, now)?;
            let Some(m) = messages::get(tx, message_id)? else {
                return Err(CorvusError::NotFound(format!("message {message_id} not found")));
            };
            if m.sender == a.as_str() {
                return Err(CorvusError::Conflict("you cannot ack your own message".into()));
            }
            if m.recipient.as_deref().is_some_and(|r| r != a.as_str()) {
                return Err(CorvusError::Conflict(
                    "that message is addressed to someone else".into(),
                ));
            }
            agents::ensure(tx, &a, now)?;
            let sid = session.map(|s| s.to_string());
            let newly = receipts::insert(tx, message_id, a.as_str(), sid.as_deref(), now)?;
            if newly {
                events::insert(
                    tx,
                    EventKind::InboxAcked,
                    &NewEvent {
                        summary: format!("{a} read message {message_id} from {}", m.sender),
                        agent_name: Some(a.as_str()),
                        session_id: sid.as_deref(),
                        thread_id: Some(&m.thread_id),
                        message_id: Some(message_id),
                        ..NewEvent::default()
                    },
                    now,
                )?;
            }
            Ok(AckResult {
                message_id,
                newly_acked: newly,
            })
        })
    }

    /// The operator's view: newest first, with read/unread-by lists. Never acks.
    pub fn list_messages(&self, req: &ListMessages) -> Result<Vec<MessageListItem>> {
        let a = req.agent.as_deref().map(|x| agent("agent", x)).transpose()?;
        let th = req.thread.as_deref().map(thread_id).transpose()?;
        let n = limit(req.limit, DEFAULT_INBOX_LIMIT);
        self.s.db().read(|tx| {
            let rows = messages::list(
                tx,
                req.before_id,
                a.as_ref().map(AgentName::as_str),
                th.as_deref(),
                req.unread_only,
                n,
            )?;
            let audience_all = messages::broadcast_audience(tx)?;
            rows.into_iter()
                .map(|m| {
                    let read_by = receipts::readers(tx, m.id)?;
                    let audience: Vec<String> = match &m.recipient {
                        Some(r) => vec![r.clone()],
                        None => audience_all.iter().filter(|x| **x != m.sender).cloned().collect(),
                    };
                    let unread_by = audience.into_iter().filter(|x| !read_by.contains(x)).collect();
                    let thread_title = threads::get(tx, &m.thread_id)?
                        .map(|t| t.title)
                        .unwrap_or_default();
                    Ok(MessageListItem {
                        message: views::message(tx, m)?,
                        thread_title,
                        read_by,
                        unread_by,
                    })
                })
                .collect()
        })
    }

    // ---------- memory ----------

    /// Appends a digest for a scope. The referenced task / thread / agent must exist.
    pub fn write_digest(&self, req: &WriteDigest) -> Result<DigestView> {
        let author = agent("author", &req.author)?;
        let scope = Scope::from_parts(req.scope.trim(), req.reference.as_deref().map(str::trim))?;
        validate_summary(&req.summary)?;
        if req.summary.chars().count() > corvus_domain::MAX_BODY_CHARS {
            return Err(CorvusError::Validation(
                "summary: at most 20000 characters".into(),
            ));
        }
        if req.covers_through.is_some_and(|c| c < 1) {
            return Err(CorvusError::Validation(
                "covers_through: must be a message id".into(),
            ));
        }
        let key = idempotency_key(&req.idempotency_key)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        self.s.db().write(|tx| {
            touch_as(tx, session.as_ref(), &author, now)?;
            agents::ensure(tx, &author, now)?;
            if let Some(k) = &key
                && let Some(d) = digests::by_idempotency_key(tx, author.as_str(), k)?
            {
                return Ok(views::digest(d));
            }
            check_scope_ref(tx, &scope)?;
            let reference = scope.reference();
            let Some(id) = digests::insert(
                tx,
                &digests::NewDigest {
                    scope: scope.kind(),
                    reference: reference.as_deref(),
                    summary: &req.summary,
                    author: author.as_str(),
                    covers_through: req.covers_through,
                    idempotency_key: key.as_deref(),
                },
                now,
            )?
            else {
                let k = key.as_deref().unwrap_or_default();
                return digests::by_idempotency_key(tx, author.as_str(), k)?
                    .map(views::digest)
                    .ok_or_else(|| CorvusError::Internal("idempotent digest vanished".into()));
            };
            events::insert(
                tx,
                EventKind::DigestWritten,
                &NewEvent {
                    summary: format!("{author} wrote a digest for {}", scope.scope_key()),
                    agent_name: Some(author.as_str()),
                    session_id: session.map(|s| s.to_string()).as_deref(),
                    task_id: match &scope {
                        Scope::Task(t) => Some(t.to_string()),
                        _ => None,
                    }
                    .as_deref(),
                    thread_id: match &scope {
                        Scope::Thread(t) => Some(t.to_string()),
                        _ => None,
                    }
                    .as_deref(),
                    ..NewEvent::default()
                },
                now,
            )?;
            digests::get(tx, id)?
                .map(views::digest)
                .ok_or_else(|| CorvusError::Internal("digest vanished".into()))
        })
    }

    /// The current digest for a scope (`None` if there is none). Unknown ref → not found.
    pub fn latest_digest(&self, scope: &str, reference: Option<&str>) -> Result<Option<DigestView>> {
        let scope = Scope::from_parts(scope.trim(), reference.map(str::trim))?;
        self.s.db().read(|tx| {
            check_scope_ref(tx, &scope)?;
            Ok(digests::latest(tx, &scope.scope_key())?.map(views::digest))
        })
    }

    /// The current digest of every scope key.
    pub fn list_latest_digests(&self) -> Result<Vec<DigestView>> {
        self.s
            .db()
            .read(|tx| Ok(digests::latest_all(tx)?.into_iter().map(views::digest).collect()))
    }

    /// Records a lightweight ADR. `supersedes` hides the older decision from default listings.
    pub fn record_decision(&self, req: &RecordDecision) -> Result<DecisionView> {
        let by = agent("decided_by", &req.decided_by)?;
        validate_title(&req.title)?;
        if req.decision.trim().is_empty() {
            return Err(CorvusError::Validation("decision: must not be empty".into()));
        }
        let decision = text("decision", &Some(req.decision.clone()), MAX_DECISION_CHARS)?.unwrap_or_default();
        let rationale = text("rationale", &req.rationale, MAX_DECISION_CHARS)?.unwrap_or_default();
        let task = req
            .task_id
            .as_deref()
            .map(|t| task_id("task_id", t))
            .transpose()?
            .map(|t| t.to_string());
        let thread = req.thread_id.as_deref().map(thread_id).transpose()?;
        let supersedes = req.supersedes.as_deref().map(decision_id).transpose()?;
        let key = idempotency_key(&req.idempotency_key)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();
        self.s.db().write(|tx| {
            touch_as(tx, session.as_ref(), &by, now)?;
            agents::ensure(tx, &by, now)?;
            if let Some(k) = &key
                && let Some(d) = decisions::by_idempotency_key(tx, by.as_str(), k)?
            {
                return Ok(views::decision(d));
            }
            if let Some(t) = &task
                && !tasks::exists(tx, t)?
            {
                return Err(CorvusError::NotFound(format!("task {t} not found")));
            }
            if let Some(t) = &thread
                && threads::get(tx, t)?.is_none()
            {
                return Err(CorvusError::NotFound(format!("thread {t} not found")));
            }
            if let Some(s) = &supersedes
                && decisions::get(tx, s)?.is_none()
            {
                return Err(CorvusError::NotFound(format!("decision {s} not found")));
            }
            let id = new_decision_id().to_string();
            decisions::insert(
                tx,
                &decisions::NewDecision {
                    id: &id,
                    title: req.title.trim(),
                    decision: &decision,
                    rationale: &rationale,
                    decided_by: by.as_str(),
                    task_id: task.as_deref(),
                    thread_id: thread.as_deref(),
                    supersedes: supersedes.as_deref(),
                    idempotency_key: key.as_deref(),
                },
                now,
            )?;
            let mut summary = format!("{by} recorded decision \"{}\"", clip(req.title.trim()));
            if supersedes.is_some() {
                summary.push_str(" (supersedes an earlier one)");
            }
            events::insert(
                tx,
                EventKind::DecisionRecorded,
                &NewEvent {
                    summary,
                    agent_name: Some(by.as_str()),
                    session_id: session.map(|s| s.to_string()).as_deref(),
                    task_id: task.as_deref(),
                    thread_id: thread.as_deref(),
                    ..NewEvent::default()
                },
                now,
            )?;
            decisions::get(tx, &id)?
                .map(views::decision)
                .ok_or_else(|| CorvusError::Internal("decision vanished".into()))
        })
    }

    pub fn list_decisions(&self, req: &ListDecisions) -> Result<Vec<DecisionView>> {
        let task = req
            .task_id
            .as_deref()
            .map(|t| task_id("task_id", t))
            .transpose()?
            .map(|t| t.to_string());
        let thread = req.thread_id.as_deref().map(thread_id).transpose()?;
        let n = limit(req.limit, 100);
        self.s.db().read(|tx| {
            Ok(
                decisions::list(tx, task.as_deref(), thread.as_deref(), req.include_superseded, n)?
                    .into_iter()
                    .map(views::decision)
                    .collect(),
            )
        })
    }

    /// `register_session` + `bootstrap` + the guidance text, in one call (two transactions: the registration,
    /// then a consistent read).
    pub fn connect(&self, req: &crate::api::RegisterSession) -> Result<crate::api::ConnectResult> {
        let session = self.s.sessions().register(req)?;
        let bootstrap = self.bootstrap(&session.agent_name)?;
        Ok(crate::api::ConnectResult {
            session,
            bootstrap,
            guidance: corvus_domain::GUIDANCE.to_owned(),
        })
    }

    /// The bootstrap context for `agent`, in one read transaction.
    pub fn bootstrap(&self, agent_name: &str) -> Result<Bootstrap> {
        let a = agent("agent", agent_name)?;
        self.s.db().read(|tx| bootstrap_in(tx, &a))
    }
}

pub(crate) fn bootstrap_in(tx: &impl Reader, a: &AgentName) -> Result<Bootstrap> {
    let task_views = |rows: Vec<tasks::TaskRow>| {
        rows.into_iter()
            .map(|r| views::task(tx, r))
            .collect::<Result<Vec<_>>>()
    };
    let msg_views = |rows: Vec<messages::MessageRow>| {
        rows.into_iter()
            .map(|m| views::message(tx, m))
            .collect::<Result<Vec<_>>>()
    };
    Ok(Bootstrap {
        agent: a.to_string(),
        routed_tasks: task_views(tasks::routed_to(tx, a.as_str(), 50)?)?,
        claimable_tasks: task_views(tasks::claimable_for(tx, a.as_str(), 20)?)?,
        unread_messages: msg_views(messages::unread(tx, a.as_str(), 0, 50)?)?,
        unresolved_questions: msg_views(messages::unresolved_questions_for(tx, a.as_str(), 50)?)?,
        digests: digests::latest_all(tx)?.into_iter().map(views::digest).collect(),
        decisions: decisions::list(tx, None, None, false, 20)?
            .into_iter()
            .map(views::decision)
            .collect(),
    })
}

/// Validation, existence checks and the insert for one message, inside the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn post_in_tx(
    tx: &WriteTx<'_>,
    thread: &str,
    from: &AgentName,
    to: Option<&AgentName>,
    kind: MessageType,
    in_reply_to: Option<i64>,
    body: &str,
    arts: &[(ArtifactRef, String)],
    key: Option<&str>,
    session: Option<&corvus_domain::SessionId>,
    now: i64,
) -> Result<MessageView> {
    touch_as(tx, session, from, now)?;
    agents::ensure(tx, from, now)?;
    if let Some(k) = key
        && let Some(existing) = messages::by_idempotency_key(tx, from.as_str(), k)?
    {
        return views::message(tx, existing);
    }
    if threads::get(tx, thread)?.is_none() {
        return Err(CorvusError::NotFound(format!("thread {thread} not found")));
    }
    if let Some(t) = to
        && !agents::exists(tx, t)?
    {
        return Err(CorvusError::NotFound(format!("agent {t} not found")));
    }
    if let Some(parent) = in_reply_to {
        match messages::get(tx, parent)? {
            None => return Err(CorvusError::NotFound(format!("message {parent} not found"))),
            Some(p) if p.thread_id != thread => {
                return Err(CorvusError::Validation(
                    "in_reply_to: a reply must stay in its thread".into(),
                ));
            }
            Some(_) => {}
        }
    }
    let sid = session.map(|s| s.to_string());
    let id = messages::insert(
        tx,
        &messages::NewMessage {
            thread_id: thread,
            sender: from.as_str(),
            recipient: to.map(AgentName::as_str),
            kind,
            in_reply_to,
            body,
            session_id: sid.as_deref(),
            idempotency_key: key,
        },
        now,
    )?;
    for (a, desc) in arts {
        artifacts::insert(
            tx,
            &new_artifact_id().to_string(),
            a,
            desc,
            artifacts::Owner::Message(id),
            now,
        )?;
    }
    let audience = to.map_or_else(|| "everyone".to_owned(), ToString::to_string);
    events::insert(
        tx,
        EventKind::MessagePosted,
        &NewEvent {
            summary: format!("{from} sent a {kind} to {audience}"),
            agent_name: Some(from.as_str()),
            session_id: sid.as_deref(),
            thread_id: Some(thread),
            message_id: Some(id),
            ..NewEvent::default()
        },
        now,
    )?;
    let row = messages::get(tx, id)?.ok_or_else(|| CorvusError::Internal("message vanished".into()))?;
    views::message(tx, row)
}

fn check_scope_ref(tx: &impl Reader, scope: &Scope) -> Result<()> {
    let found = match scope {
        Scope::Global => true,
        Scope::Task(t) => tasks::exists(tx, &t.to_string())?,
        Scope::Thread(t) => threads::get(tx, &t.to_string())?.is_some(),
        Scope::Agent(a) => agents::exists(tx, a)?,
    };
    if found {
        Ok(())
    } else {
        Err(CorvusError::NotFound(format!("{} not found", scope.scope_key())))
    }
}

fn thread_id(s: &str) -> Result<String> {
    corvus_domain::ThreadId::parse(s.trim())
        .map(|t| t.to_string())
        .map_err(|_| CorvusError::Validation("thread_id: must be a UUID".into()))
}

fn decision_id(s: &str) -> Result<String> {
    corvus_domain::DecisionId::parse(s.trim())
        .map(|t| t.to_string())
        .map_err(|_| CorvusError::Validation("supersedes: must be a UUID".into()))
}
