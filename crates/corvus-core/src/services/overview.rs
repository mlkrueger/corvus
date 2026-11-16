//! Read-only aggregations (brief §4.5): `stats`, `overview`, `health`, `list_events`. Each is one read
//! transaction.

use std::collections::BTreeMap;

use std::collections::BTreeSet;

use corvus_domain::{ConversationRef, Result, TaskStatus, Tool};

use crate::api::{
    AgentStatus, EventView, Health, ListEvents, Overview, OverviewSession, Stats, TaskColumns, Thresholds,
};
use crate::clock::Timestamp;
use crate::db::{Reader, migrator::SCHEMA_VERSION};
use crate::delivery::reach::{self, Holders};
use crate::services::sessions::conversation_view;
use crate::services::{Services, agent, views};
use crate::store::{agents, claims, conversations, decisions, events, messages, sessions, tasks};

/// Overview limits ([defaults #18](../../../../spec/system/11-defaults.md)).
const COLUMN_LIMIT: u32 = 50;
const EVENT_LIMIT: u32 = 100;
const DECISION_LIMIT: u32 = 20;
/// Ended sessions stay on the overview this long.
const RECENTLY_ENDED_MS: i64 = 24 * 3600 * 1000;

#[derive(Debug, Clone, Copy)]
pub struct OverviewService<'a> {
    pub(super) s: &'a Services,
}

impl OverviewService<'_> {
    pub fn stats(&self) -> Result<Stats> {
        let now = self.s.now();
        self.s.db().read(|tx| stats_in(tx, now))
    }

    pub fn health(&self) -> Result<Health> {
        let v = self.s.db().read(|tx| {
            tx.sql()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .map_err(crate::db::sql_err)
        })?;
        Ok(Health {
            ok: v == SCHEMA_VERSION,
            schema_version: v,
        })
    }

    pub fn list_events(&self, req: &ListEvents) -> Result<Vec<EventView>> {
        let n = req.limit.unwrap_or(100).clamp(1, 500);
        self.s.db().read(|tx| {
            Ok(events::list(tx, req.before_id, n)?
                .into_iter()
                .map(views::event)
                .collect())
        })
    }

    /// Held watcher and waiter locks. File IO, so always before the transaction, never inside it.
    fn holders(&self) -> Holders {
        self.s.state_dir().map(reach::scan).unwrap_or_default()
    }

    /// Every agent that has had a session, a conversation or unread direct mail, with whether anything can
    /// deliver its mail right now (system/06 "Reachable").
    pub fn agents(&self) -> Result<Vec<AgentStatus>> {
        let holders = self.holders();
        self.s.db().read(|tx| agents_in(tx, &holders))
    }

    /// One agent's status. `NotFound` for an agent Corvus has never seen.
    pub fn agent_status(&self, name: &str) -> Result<AgentStatus> {
        let name = agent("agent", name)?;
        let holders = self.holders();
        self.s.db().read(|tx| {
            if !agents::exists(tx, &name)? {
                return Err(corvus_domain::CorvusError::NotFound(format!(
                    "agent {name} not found"
                )));
            }
            let reachable = reachable_agents(tx, &holders)?;
            let unread = messages::unread_direct(tx, name.as_str())?.len();
            Ok(AgentStatus {
                reachable: reachable.contains(name.as_str()),
                unread_direct: i64::try_from(unread).unwrap_or(i64::MAX),
                conversations: conversations::for_agent(tx, name.as_str())?
                    .into_iter()
                    .map(conversation_view)
                    .collect(),
                name: name.to_string(),
            })
        })
    }

    /// Everything the dashboard shows, from one consistent snapshot.
    pub fn overview(&self) -> Result<Overview> {
        let holders = self.holders();
        let now = self.s.now();
        let settings = self.s.settings();
        let operator = self.s.operator_name().to_string();
        self.s.db().read(|tx| {
            let stats = stats_in(tx, now)?;
            let mut presence_counts = BTreeMap::new();
            let mut list = Vec::new();
            for row in sessions::list(tx, true, None)? {
                if row.ended_at.is_some_and(|e| now - e > RECENTLY_ENDED_MS) {
                    continue;
                }
                let unread = stats.unread_by_agent.get(&row.agent_name).copied().unwrap_or(0);
                let v = views::session(row, now, &settings)?;
                if v.ended_at.is_none() {
                    *presence_counts.entry(v.presence.as_str().to_owned()).or_insert(0) += 1;
                }
                list.push(OverviewSession { session: v, unread });
            }
            let column = |st: TaskStatus| -> Result<Vec<_>> {
                tasks::recent_with_status(tx, st, COLUMN_LIMIT)?
                    .into_iter()
                    .map(|r| views::task(tx, r))
                    .collect()
            };
            let ms_to_s = |ms: i64| ms / 1000;
            Ok(Overview {
                generated_at: Timestamp(now),
                operator_name: operator.clone(),
                thresholds: Thresholds {
                    active_seconds: ms_to_s(settings.active_ms),
                    idle_seconds: ms_to_s(settings.idle_ms),
                    lease_seconds: ms_to_s(settings.lease_ms),
                    auto_end_seconds: ms_to_s(settings.auto_end_ms),
                },
                presence_counts,
                sessions: list,
                agents: agents_in(tx, &holders)?,
                tasks: TaskColumns {
                    open: column(TaskStatus::Open)?,
                    claimed: column(TaskStatus::Claimed)?,
                    done: column(TaskStatus::Done)?,
                    cancelled: column(TaskStatus::Cancelled)?,
                },
                blocked_by_cancelled: tasks::blocked_by_cancelled_count(tx)?,
                events: events::list(tx, None, EVENT_LIMIT)?
                    .into_iter()
                    .map(views::event)
                    .collect(),
                decisions: decisions::list(tx, None, None, false, DECISION_LIMIT)?
                    .into_iter()
                    .map(views::decision)
                    .collect(),
                stats,
            })
        })
    }
}

/// Agents something can deliver to: an open session, a waiter whose conversation is linked to the agent, or
/// a watcher.
fn reachable_agents(tx: &impl Reader, holders: &Holders) -> Result<BTreeSet<String>> {
    let mut out: BTreeSet<String> = holders.watchers.clone();
    out.extend(sessions::list(tx, false, None)?.into_iter().map(|s| s.agent_name));
    for id in &holders.waiting_conversations {
        if let Ok(conversation) = ConversationRef::new(Tool::ClaudeCode, id)
            && let Some(link) = conversations::get(tx, &conversation)?
        {
            out.insert(link.agent_name);
        }
    }
    Ok(out)
}

fn agents_in(tx: &impl Reader, holders: &Holders) -> Result<Vec<AgentStatus>> {
    let reachable = reachable_agents(tx, holders)?;
    let unread: BTreeMap<String, i64> = messages::unread_direct_counts(tx)?.into_iter().collect();
    let with_session: BTreeSet<String> = messages::broadcast_audience(tx)?.into_iter().collect();
    let mut out = Vec::new();
    for name in agents::all(tx)? {
        let conversations: Vec<_> = conversations::for_agent(tx, &name)?
            .into_iter()
            .map(conversation_view)
            .collect();
        let unread_direct = unread.get(&name).copied().unwrap_or(0);
        if conversations.is_empty() && unread_direct == 0 && !with_session.contains(&name) {
            continue; // only ever a sender or task author: nothing to show
        }
        out.push(AgentStatus {
            reachable: reachable.contains(&name),
            unread_direct,
            conversations,
            name,
        });
    }
    Ok(out)
}

pub(crate) fn stats_in(tx: &impl Reader, now: i64) -> Result<Stats> {
    let counts: BTreeMap<String, i64> = tasks::status_counts(tx)?.into_iter().collect();
    let (active, expired) = claims::lease_counts(tx, now)?;
    let unread: BTreeMap<String, i64> = messages::unread_counts(tx)?.into_iter().collect();
    Ok(Stats {
        open_tasks: counts.get("open").copied().unwrap_or(0),
        claimed_tasks: counts.get("claimed").copied().unwrap_or(0),
        active_leases: active,
        expired_leases: expired,
        unread_total: unread.values().sum(),
        unread_by_agent: unread,
    })
}
