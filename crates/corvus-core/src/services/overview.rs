//! Read-only aggregations (brief §4.5): `stats`, `overview`, `health`, `list_events`. Each is one read
//! transaction.

use std::collections::BTreeMap;

use corvus_domain::{Result, TaskStatus};

use crate::api::{EventView, Health, ListEvents, Overview, OverviewSession, Stats, TaskColumns, Thresholds};
use crate::clock::Timestamp;
use crate::db::{Reader, migrator::SCHEMA_VERSION};
use crate::services::{Services, views};
use crate::store::{claims, decisions, events, messages, sessions, tasks};

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

    /// Everything the dashboard shows, from one consistent snapshot.
    pub fn overview(&self) -> Result<Overview> {
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
