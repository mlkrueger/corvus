//! Store rows → API views.

use corvus_domain::{Result, presence};

use crate::api::{ArtifactView, ClaimView, Holder, SessionView, TaskView};
use crate::clock::Timestamp;
use crate::db::Reader;
use crate::services::Settings;
use crate::store::artifacts::{self, Owner};
use crate::store::claims::ClaimRow;
use crate::store::sessions::SessionRow;
use crate::store::{claims, tasks, threads};

fn ts(ms: i64) -> Timestamp {
    Timestamp(ms)
}

fn parse<T: std::str::FromStr<Err = corvus_domain::CorvusError>>(s: &str) -> Result<T> {
    s.parse()
}

pub(crate) fn session(row: SessionRow, now: i64, settings: &Settings) -> Result<SessionView> {
    let p = presence(
        row.ended_at.is_some(),
        row.last_seen_at,
        row.current_task.is_some(),
        now,
        settings.active_ms,
        settings.idle_ms,
    );
    Ok(SessionView {
        metadata: serde_json::from_str(&row.metadata).unwrap_or(serde_json::Value::Null),
        end_reason: row.end_reason.as_deref().map(parse).transpose()?,
        id: row.id,
        agent_name: row.agent_name,
        client_name: row.client_name,
        name_generated: row.name_generated,
        started_at: ts(row.started_at),
        last_seen_at: ts(row.last_seen_at),
        ended_at: row.ended_at.map(ts),
        presence: p,
        current_task: row.current_task,
        lease_expires_at: row.lease_expires_at.map(ts),
    })
}

pub(crate) fn claim(c: ClaimRow) -> Result<ClaimView> {
    Ok(ClaimView {
        end_reason: c.end_reason.as_deref().map(parse).transpose()?,
        id: c.id,
        task_id: c.task_id,
        session_id: c.session_id,
        agent_name: c.agent_name,
        claimed_at: ts(c.claimed_at),
        lease_expires_at: ts(c.lease_expires_at),
        released_at: c.released_at.map(ts),
    })
}

/// The full task view: thread, dependencies, artifacts, derived flags and holder.
pub(crate) fn task(r: &impl Reader, row: tasks::TaskRow) -> Result<TaskView> {
    let thread_id = threads::for_task(r, &row.id)?;
    let depends_on = tasks::dependencies(r, &row.id)?;
    let (blocked, blocked_by_cancelled) = tasks::flags(r, &row.id)?;
    let holder = claims::active_for_task(r, &row.id)?.map(|c| Holder {
        agent_name: c.agent_name,
        session_id: c.session_id,
        claim_id: c.id,
        claimed_at: ts(c.claimed_at),
        lease_expires_at: ts(c.lease_expires_at),
    });
    let artifacts = artifacts::list(r, Owner::Task(&row.id))?
        .into_iter()
        .map(|a| {
            Ok(ArtifactView {
                kind: parse(&a.kind)?,
                id: a.id,
                path: a.path,
                commit: a.commit_hash,
                description: a.description,
                created_at: ts(a.created_at),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TaskView {
        status: parse(&row.status)?,
        id: row.id,
        title: row.title,
        body: row.body,
        priority: row.priority,
        pool: row.pool,
        recipient_agent: row.recipient_agent,
        creator: row.creator,
        result_summary: row.result_summary,
        cancel_reason: row.cancel_reason,
        cancelled_by: row.cancelled_by,
        created_at: ts(row.created_at),
        updated_at: ts(row.updated_at),
        completed_at: row.completed_at.map(ts),
        cancelled_at: row.cancelled_at.map(ts),
        thread_id,
        depends_on,
        artifacts,
        blocked,
        blocked_by_cancelled,
        holder,
    })
}

pub(crate) fn task_by_id(r: &impl Reader, id: &str) -> Result<TaskView> {
    match tasks::get(r, id)? {
        Some(row) => task(r, row),
        None => Err(corvus_domain::CorvusError::NotFound(format!(
            "task {id} not found"
        ))),
    }
}
