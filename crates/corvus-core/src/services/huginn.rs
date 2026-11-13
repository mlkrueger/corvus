//! Huginn: tasks, claims, leases, cancellation (brief §4.2, system/03 "Lock protocol", D0017).
//!
//! Every operation is one `BEGIN IMMEDIATE` transaction, so operations are strictly serial across threads and
//! processes. Statement order inside each follows the brief's lock order (session → claims → tasks) for
//! readability; safety comes from the single write lock and the partial unique indexes on `claims`.

use std::collections::HashSet;

use corvus_domain::{
    ArtifactRef, CorvusError, EndReason, EventKind, MAX_BODY_CHARS, MessageType, Result, TaskStatus,
    validate_task_body, validate_title,
};

use crate::api::{
    CancelResult, CancelTask, ClaimResult, ClaimTask, CompleteTask, CreateTask, ExpireResult, Holder,
    ListTasks, TaskView,
};
use crate::clock::Timestamp;
use crate::db::WriteTx;
use crate::ids::{new_artifact_id, new_claim_id, new_task_id, new_thread_id};
use crate::services::{Services, agent, idempotency_key, label, session_id, task_id, text, touch, views};
use crate::store::claims::ClaimRow;
use crate::store::events::{self, NewEvent, clip};
use crate::store::{agents, artifacts, claims, messages, tasks, threads};

const MAX_DEPENDENCIES: usize = 100;
const MAX_ARTIFACTS: usize = 100;
const MAX_REASON_CHARS: usize = 2_000;
const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_LIST_LIMIT: u32 = 500;

#[derive(Debug, Clone, Copy)]
pub struct Huginn<'a> {
    pub(super) s: &'a Services,
}

/// Which lease operation (they share their ownership rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseOp {
    Renew,
    Release,
    Complete,
}

impl Huginn<'_> {
    /// Creates a task with its thread, dependencies and artifacts. Idempotent on `(creator, key)`: a replay
    /// returns the original task (in its current state) and creates nothing.
    pub fn create_task(&self, req: &CreateTask) -> Result<TaskView> {
        let creator = agent("creator", &req.creator)?;
        validate_title(&req.title)?;
        let title = req.title.trim().to_owned();
        let body = req.body.clone().unwrap_or_default();
        validate_task_body(&body)?;
        let pool = label("pool", req.pool.as_deref().unwrap_or("default"), 64)?;
        let recipient = req.recipient.as_deref().map(str::trim).filter(|r| !r.is_empty());
        let recipient = recipient.map(|r| agent("recipient", r)).transpose()?;
        if req.depends_on.len() > MAX_DEPENDENCIES {
            return Err(CorvusError::Validation(format!(
                "depends_on: at most {MAX_DEPENDENCIES}"
            )));
        }
        let mut deps = Vec::new();
        for d in &req.depends_on {
            let id = task_id("depends_on", d)?.to_string();
            if !deps.contains(&id) {
                deps.push(id);
            }
        }
        if req.artifacts.len() > MAX_ARTIFACTS {
            return Err(CorvusError::Validation(format!(
                "artifacts: at most {MAX_ARTIFACTS}"
            )));
        }
        let arts = req
            .artifacts
            .iter()
            .map(|a| {
                let r = ArtifactRef::new(a.kind, a.path.clone(), a.commit.clone())?;
                let d = text("artifact description", &a.description, 2_000)?.unwrap_or_default();
                Ok((r, d))
            })
            .collect::<Result<Vec<_>>>()?;
        let key = idempotency_key(&req.idempotency_key)?;
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();

        self.s.db().write(|tx| {
            if let Some(sid) = &session {
                let owner = touch(tx, sid, now)?;
                if owner != creator {
                    return Err(CorvusError::Conflict(format!(
                        "session belongs to {owner}, not {creator}"
                    )));
                }
            }
            agents::ensure(tx, &creator, now)?;
            if let Some(k) = &key
                && let Some(existing) = tasks::by_idempotency_key(tx, creator.as_str(), k)?
            {
                return views::task(tx, existing);
            }
            if let Some(r) = &recipient
                && !agents::exists(tx, r)?
            {
                return Err(CorvusError::NotFound(format!("agent {r} not found")));
            }
            for d in &deps {
                if !tasks::exists(tx, d)? {
                    return Err(CorvusError::NotFound(format!("dependency task {d} not found")));
                }
            }
            let id = new_task_id().to_string();
            let inserted = tasks::insert(
                tx,
                &tasks::NewTask {
                    id: &id,
                    title: &title,
                    body: &body,
                    priority: req.priority.unwrap_or(0),
                    pool: &pool,
                    recipient_agent: recipient.as_ref().map(|r| r.as_str()),
                    creator: creator.as_str(),
                    idempotency_key: key.as_deref(),
                },
                now,
            )?;
            if !inserted {
                let k = key.as_deref().unwrap_or_default();
                let existing = tasks::by_idempotency_key(tx, creator.as_str(), k)?
                    .ok_or_else(|| CorvusError::Internal("idempotent task vanished".into()))?;
                return views::task(tx, existing);
            }
            for d in &deps {
                tasks::add_dependency(tx, &id, d)?;
            }
            let thread = new_thread_id().to_string();
            threads::insert_for_task(tx, &thread, &id, &title, creator.as_str(), now)?;
            for (a, desc) in &arts {
                artifacts::insert(
                    tx,
                    &new_artifact_id().to_string(),
                    a,
                    desc,
                    artifacts::Owner::Task(&id),
                    now,
                )?;
            }
            events::insert(
                tx,
                EventKind::TaskCreated,
                &NewEvent {
                    summary: format!("{creator} created task \"{}\" (pool {pool})", clip(&title)),
                    agent_name: Some(creator.as_str()),
                    session_id: session.map(|s| s.to_string()).as_deref(),
                    task_id: Some(&id),
                    thread_id: Some(&thread),
                    ..NewEvent::default()
                },
                now,
            )?;
            views::task_by_id(tx, &id)
        })
    }

    /// Claims the best claimable task for the session's agent, after reaping expired leases. Idempotent per
    /// session: a session that already holds a claim gets that claim back. Both fields `None` when nothing is
    /// claimable.
    pub fn claim_task(&self, req: &ClaimTask) -> Result<ClaimResult> {
        let sid = session_id(&req.session_id)?;
        let pool = req.pool.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let pool = pool.map(|p| label("pool", p, 64)).transpose()?;
        let now = self.s.now();
        let lease = self.s.settings().lease_ms;

        self.s.db().write(|tx| {
            let agent_name = touch(tx, &sid, now)?;
            let sid_s = sid.to_string();
            if let Some(c) = claims::active_for_session(tx, &sid_s)? {
                let task = views::task_by_id(tx, &c.task_id)?;
                return Ok(ClaimResult {
                    task: Some(task),
                    claim: Some(views::claim(c)?),
                });
            }
            reap(tx, now)?;
            let Some(task_id) = tasks::next_claimable(tx, pool.as_deref(), agent_name.as_str())? else {
                return Ok(ClaimResult {
                    task: None,
                    claim: None,
                });
            };
            let c = ClaimRow {
                id: new_claim_id().to_string(),
                task_id: task_id.clone(),
                session_id: sid_s.clone(),
                agent_name: agent_name.to_string(),
                claimed_at: now,
                lease_expires_at: now.saturating_add(lease),
                released_at: None,
                end_reason: None,
            };
            claims::insert(tx, &c)?;
            tasks::set_status(tx, &task_id, TaskStatus::Claimed, now)?;
            let task = views::task_by_id(tx, &task_id)?;
            events::insert(
                tx,
                EventKind::TaskClaimed,
                &NewEvent {
                    summary: format!("{agent_name} claimed \"{}\"", clip(&task.title)),
                    agent_name: Some(agent_name.as_str()),
                    session_id: Some(&sid_s),
                    task_id: Some(&task_id),
                    ..NewEvent::default()
                },
                now,
            )?;
            Ok(ClaimResult {
                task: Some(task),
                claim: Some(views::claim(c)?),
            })
        })
    }

    /// Extends the holder's lease to `now + lease`.
    pub fn renew_lease(&self, task: &str, session: &str) -> Result<TaskView> {
        self.lease_op(LeaseOp::Renew, task, session, None)
    }

    /// Gives the task back (it reopens). A replay by the same session is a no-op.
    pub fn release_task(&self, task: &str, session: &str) -> Result<TaskView> {
        self.lease_op(LeaseOp::Release, task, session, None)
    }

    /// Marks the task `done` (final). A replay by the same session is a no-op.
    pub fn complete_task(&self, task: &str, req: &CompleteTask) -> Result<TaskView> {
        let summary = text("result_summary", &req.result_summary, MAX_BODY_CHARS)?;
        self.lease_op(LeaseOp::Complete, task, &req.session_id, summary)
    }

    fn lease_op(&self, op: LeaseOp, task: &str, session: &str, summary: Option<String>) -> Result<TaskView> {
        let tid = task_id("task_id", task)?.to_string();
        let sid = session_id(session)?;
        let now = self.s.now();
        let lease = self.s.settings().lease_ms;

        self.s.db().write(|tx| {
            let agent_name = touch(tx, &sid, now)?;
            let sid_s = sid.to_string();
            let Some(t) = tasks::get(tx, &tid)? else {
                return Err(CorvusError::NotFound(format!("task {tid} not found")));
            };
            let active = claims::active_for_task(tx, &tid)?;
            if let Some(c) = active.filter(|c| c.session_id == sid_s) {
                // The holder. An expired-but-unreaped lease still counts: whichever transaction runs first wins.
                let (kind, verb) = match op {
                    LeaseOp::Renew => {
                        claims::set_lease(tx, &c.id, now.saturating_add(lease))?;
                        (EventKind::TaskRenewed, "renewed the lease on")
                    }
                    LeaseOp::Release => {
                        claims::end(tx, &c.id, EndReason::Released, now)?;
                        tasks::set_status(tx, &tid, TaskStatus::Open, now)?;
                        (EventKind::TaskReleased, "released")
                    }
                    LeaseOp::Complete => {
                        claims::end(tx, &c.id, EndReason::Completed, now)?;
                        tasks::complete(tx, &tid, summary.as_deref(), now)?;
                        (EventKind::TaskCompleted, "completed")
                    }
                };
                events::insert(
                    tx,
                    kind,
                    &NewEvent {
                        summary: format!("{agent_name} {verb} \"{}\"", clip(&t.title)),
                        agent_name: Some(agent_name.as_str()),
                        session_id: Some(&sid_s),
                        task_id: Some(&tid),
                        ..NewEvent::default()
                    },
                    now,
                )?;
                return views::task_by_id(tx, &tid);
            }
            if t.status == TaskStatus::Cancelled.as_str() {
                let reason = t.cancel_reason.as_deref().unwrap_or("no reason given");
                return Err(CorvusError::Conflict(format!("task was cancelled: {reason}")));
            }
            let replay_reason = match op {
                LeaseOp::Renew => None,
                LeaseOp::Release => Some(EndReason::Released),
                LeaseOp::Complete => Some(EndReason::Completed),
            };
            if let Some(want) = replay_reason
                && let Some(prev) = claims::latest_for(tx, &tid, &sid_s)?
                && prev.end_reason.as_deref() == Some(want.as_str())
            {
                return views::task_by_id(tx, &tid);
            }
            Err(CorvusError::Conflict(
                match claims::latest_for(tx, &tid, &sid_s)? {
                    Some(prev) if prev.end_reason.as_deref() == Some(EndReason::Expired.as_str()) => {
                        "your lease on this task expired; claim again".to_owned()
                    }
                    _ if claims::active_for_task(tx, &tid)?.is_some() => {
                        "task is held by another session".to_owned()
                    }
                    _ => "this session does not hold the task".to_owned(),
                },
            ))
        })
    }

    /// Ends every expired claim (reason `expired`) and reopens its task. Returns how many.
    pub fn expire_leases(&self) -> Result<ExpireResult> {
        let now = self.s.now();
        let expired = self.s.db().write(|tx| reap(tx, now))?;
        Ok(ExpireResult { expired })
    }

    /// Cancels an open or claimed task (and, with `cascade`, every transitive dependent that is not done).
    /// The active claimant of each cancelled task gets a `status` message in the task's thread. Idempotent.
    pub fn cancel_task(&self, task: &str, req: &CancelTask) -> Result<CancelResult> {
        let tid = task_id("task_id", task)?.to_string();
        let by = agent("by", &req.by)?;
        let reason = text("reason", &req.reason, MAX_REASON_CHARS)?.map(|r| r.trim().to_owned());
        let reason = reason.filter(|r| !r.is_empty());
        let session = req.session_id.as_deref().map(session_id).transpose()?;
        let now = self.s.now();

        self.s.db().write(|tx| {
            if let Some(sid) = &session {
                let owner = touch(tx, sid, now)?;
                if owner != by {
                    return Err(CorvusError::Conflict(format!("session belongs to {owner}, not {by}")));
                }
            }
            agents::ensure(tx, &by, now)?;
            let Some(root) = tasks::get(tx, &tid)? else {
                return Err(CorvusError::NotFound(format!("task {tid} not found")));
            };
            if root.status == TaskStatus::Done.as_str() {
                return Err(CorvusError::Conflict("done tasks cannot be cancelled".into()));
            }
            let mut targets = vec![tid.clone()];
            if req.cascade {
                targets.extend(tasks::dependents(tx, &tid)?);
            }
            let mut seen = HashSet::new();
            let mut cancelled = Vec::new();
            for id in targets {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Some(t) = tasks::get(tx, &id)? else { continue };
                if t.status == TaskStatus::Done.as_str() || t.status == TaskStatus::Cancelled.as_str() {
                    continue;
                }
                let claim = claims::active_for_task(tx, &id)?;
                if let Some(c) = &claim {
                    claims::end(tx, &c.id, EndReason::Cancelled, now)?;
                }
                tasks::cancel(tx, &id, by.as_str(), reason.as_deref(), now)?;
                let thread = threads::for_task(tx, &id)?;
                let mut message_id = None;
                if let (Some(c), Some(th)) = (&claim, &thread) {
                    let body = format!(
                        "Task \"{}\" was cancelled by {by}: {}. Stop working on it; renew and complete will now \
                         fail.",
                        t.title,
                        reason.as_deref().unwrap_or("no reason given")
                    );
                    message_id = Some(messages::insert(
                        tx,
                        &messages::NewMessage {
                            thread_id: th,
                            sender: by.as_str(),
                            recipient: Some(&c.agent_name),
                            kind: MessageType::Status,
                            in_reply_to: None,
                            body: &body,
                            session_id: None,
                            idempotency_key: None,
                        },
                        now,
                    )?);
                }
                let mut summary = format!("{by} cancelled \"{}\"", clip(&t.title));
                if id != tid {
                    summary.push_str(" (cascade)");
                }
                if let Some(c) = &claim {
                    summary.push_str(&format!("; {} was notified", c.agent_name));
                }
                events::insert(
                    tx,
                    EventKind::TaskCancelled,
                    &NewEvent {
                        summary,
                        agent_name: Some(by.as_str()),
                        session_id: session.map(|s| s.to_string()).as_deref(),
                        task_id: Some(&id),
                        thread_id: thread.as_deref(),
                        message_id,
                    },
                    now,
                )?;
                cancelled.push(id);
            }
            Ok(CancelResult { task: views::task_by_id(tx, &tid)?, cancelled })
        })
    }

    /// Appends text to an open task's body (work sources use this). Truncates at the body cap.
    pub fn append_to_task(&self, task: &str, addition: &str) -> Result<TaskView> {
        let tid = task_id("task_id", task)?.to_string();
        if addition.trim().is_empty() {
            return Err(CorvusError::Validation("text: must not be empty".into()));
        }
        let now = self.s.now();
        self.s.db().write(|tx| {
            let Some(t) = tasks::get(tx, &tid)? else {
                return Err(CorvusError::NotFound(format!("task {tid} not found")));
            };
            if t.status != TaskStatus::Open.as_str() {
                return Err(CorvusError::Conflict(format!(
                    "task is {}; only open tasks can be appended to",
                    t.status
                )));
            }
            let joined = if t.body.is_empty() {
                addition.to_owned()
            } else {
                format!("{}\n\n{addition}", t.body)
            };
            let body: String = joined.chars().take(MAX_BODY_CHARS).collect();
            tasks::set_body(tx, &tid, &body, now)?;
            events::insert(
                tx,
                EventKind::TaskAppended,
                &NewEvent {
                    summary: format!("text appended to \"{}\"", clip(&t.title)),
                    task_id: Some(&tid),
                    ..NewEvent::default()
                },
                now,
            )?;
            views::task_by_id(tx, &tid)
        })
    }

    /// The agent and session holding the task, if any.
    pub fn claimant_of(&self, task: &str) -> Result<Option<Holder>> {
        let tid = task_id("task_id", task)?.to_string();
        self.s.db().read(|tx| {
            if !tasks::exists(tx, &tid)? {
                return Err(CorvusError::NotFound(format!("task {tid} not found")));
            }
            Ok(claims::active_for_task(tx, &tid)?.map(|c| Holder {
                agent_name: c.agent_name,
                session_id: c.session_id,
                claim_id: c.id,
                claimed_at: Timestamp(c.claimed_at),
                lease_expires_at: Timestamp(c.lease_expires_at),
            }))
        })
    }

    pub fn get_task(&self, task: &str) -> Result<TaskView> {
        let tid = task_id("task_id", task)?.to_string();
        self.s.db().read(|tx| views::task_by_id(tx, &tid))
    }

    /// Newest first; `agent` matches creator, recipient or holder.
    pub fn list_tasks(&self, req: &ListTasks) -> Result<Vec<TaskView>> {
        let a = req.agent.as_deref().map(|a| agent("agent", a)).transpose()?;
        let limit = req.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
        self.s.db().read(|tx| {
            tasks::list(tx, req.status, a.as_ref().map(|a| a.as_str()), limit)?
                .into_iter()
                .map(|row| views::task(tx, row))
                .collect()
        })
    }
}

/// Reaping: every active claim with `lease_expires_at <= now` ends `expired` and its task reopens.
fn reap(tx: &WriteTx<'_>, now: i64) -> Result<usize> {
    let expired = claims::expired(tx, now)?;
    for c in &expired {
        claims::end(tx, &c.id, EndReason::Expired, now)?;
        tasks::set_status(tx, &c.task_id, TaskStatus::Open, now)?;
        events::insert(
            tx,
            EventKind::LeaseExpired,
            &NewEvent {
                summary: format!("{}'s lease expired; task reopened", c.agent_name),
                agent_name: Some(&c.agent_name),
                session_id: Some(&c.session_id),
                task_id: Some(&c.task_id),
                ..NewEvent::default()
            },
            now,
        )?;
    }
    Ok(expired.len())
}

/// A task created inside the caller's write transaction (work sources): agent upsert, idempotency on
/// `(creator, key)`, the task's thread and one `task.created` event. Returns the task id and whether it was new.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_task_tx(
    tx: &WriteTx<'_>,
    creator: &corvus_domain::AgentName,
    title: &str,
    body: &str,
    priority: i64,
    pool: &str,
    key: Option<&str>,
    now: i64,
) -> Result<(String, bool)> {
    agents::ensure(tx, creator, now)?;
    if let Some(k) = key
        && let Some(existing) = tasks::by_idempotency_key(tx, creator.as_str(), k)?
    {
        return Ok((existing.id, false));
    }
    let id = new_task_id().to_string();
    let inserted = tasks::insert(
        tx,
        &tasks::NewTask {
            id: &id,
            title,
            body,
            priority,
            pool,
            recipient_agent: None,
            creator: creator.as_str(),
            idempotency_key: key,
        },
        now,
    )?;
    if !inserted {
        let existing = tasks::by_idempotency_key(tx, creator.as_str(), key.unwrap_or_default())?
            .ok_or_else(|| CorvusError::Internal("idempotent task vanished".into()))?;
        return Ok((existing.id, false));
    }
    let thread = new_thread_id().to_string();
    threads::insert_for_task(tx, &thread, &id, title, creator.as_str(), now)?;
    events::insert(
        tx,
        EventKind::TaskCreated,
        &NewEvent {
            summary: format!("{creator} created task \"{}\" (pool {pool})", clip(title)),
            agent_name: Some(creator.as_str()),
            task_id: Some(&id),
            thread_id: Some(&thread),
            ..NewEvent::default()
        },
        now,
    )?;
    Ok((id, true))
}

/// Appends to an open task's body inside the caller's transaction (no event: the caller records one).
pub(crate) fn append_tx(tx: &WriteTx<'_>, task_id: &str, addition: &str, now: i64) -> Result<()> {
    let Some(t) = tasks::get(tx, task_id)? else {
        return Err(CorvusError::NotFound(format!("task {task_id} not found")));
    };
    let joined = if t.body.is_empty() {
        addition.to_owned()
    } else {
        format!("{}\n\n{addition}", t.body)
    };
    let body: String = joined.chars().take(MAX_BODY_CHARS).collect();
    tasks::set_body(tx, task_id, &body, now)
}
