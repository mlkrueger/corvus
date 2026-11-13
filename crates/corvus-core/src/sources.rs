//! The work-source engine ([system/07](../../../spec/system/07-work-sources.md), D0011, D0012).
//!
//! Adapters (`corvus-jira`, `corvus-linear`) implement [`WorkSource`]: pure translators that return plain text
//! and mapped priorities and never touch the database. The engine owns the pass:
//!
//! 1. single flight — an in-process async mutex per source plus an OS file lock on
//!    `<state>/run/source-<id>.lock`, so a `corvus <source>-sync` in another process cannot interleave;
//! 2. fetch everything (search, then comments for the tickets that need them) — any failure aborts the pass,
//!    records `last_error` and writes nothing else;
//! 3. reconcile in short write transactions (no network inside any of them).
//!
//! No tracker-specific type appears here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corvus_domain::sources::{
    advance_mark, comments_text, followup_body, is_new_comment, task_body, task_title,
};
use corvus_domain::{
    AgentName, Comment, CorvusError, EventKind, MessageType, Result, SourceFilter, SourceId, StatusCategory,
    TaskStatus, Ticket, TicketKey,
};

use crate::api::{QueueTicket, SourceSummary, SourceTicketView, SourceView, SyncResult, SyncStats, TaskView};
use crate::clock::Timestamp;
use crate::db::WriteTx;
use crate::services::Services;
use crate::services::huginn::{append_tx, create_task_tx};
use crate::services::muninn::post_in_tx;
use crate::store::events::{self, NewEvent, clip};
use crate::store::sources::{self as store, LinkRow, TicketRow};
use crate::store::{claims, threads};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long `known_statuses` is cached per source.
const STATUS_CACHE: Duration = Duration::from_secs(600);

/// A read-only tracker feed. Implementations live in the adapter crates.
pub trait WorkSource: Send + Sync + std::fmt::Debug {
    fn id(&self) -> SourceId;
    /// Display only (e.g. the Jira site host).
    fn site(&self) -> String;
    fn default_filter(&self) -> SourceFilter;
    /// Pool for created tasks.
    fn pool(&self) -> String;
    fn poll_interval(&self) -> Duration;
    /// Account / user ids whose comments are not work.
    fn ignored_authors(&self) -> Vec<String>;
    /// Every ticket matching `filter` (assigned to the API user), all pages — or an error.
    fn search<'a>(&'a self, filter: &'a SourceFilter, now_ms: i64) -> BoxFuture<'a, Result<Vec<Ticket>>>;
    /// Every comment of a ticket, all pages — or an error.
    fn comments<'a>(&'a self, key: &'a TicketKey) -> BoxFuture<'a, Result<Vec<Comment>>>;
    /// Status names for the filter form's suggestions.
    fn known_statuses(&self) -> BoxFuture<'_, Result<Vec<String>>>;
}

struct Entry {
    source: Arc<dyn WorkSource>,
    flight: tokio::sync::Mutex<()>,
    statuses: Mutex<Option<(Instant, Vec<String>)>>,
}

/// The configured sources of this process.
pub struct Sources {
    services: Services,
    state_dir: PathBuf,
    entries: BTreeMap<SourceId, Entry>,
}

impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sources")
            .field("enabled", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Sources {
    pub fn new(services: Services, state_dir: PathBuf, sources: Vec<Arc<dyn WorkSource>>) -> Self {
        let entries = sources
            .into_iter()
            .map(|s| {
                (
                    s.id(),
                    Entry {
                        source: s,
                        flight: tokio::sync::Mutex::new(()),
                        statuses: Mutex::new(None),
                    },
                )
            })
            .collect();
        Self {
            services,
            state_dir,
            entries,
        }
    }

    /// No sources configured.
    pub fn none(services: Services, state_dir: PathBuf) -> Self {
        Self::new(services, state_dir, Vec::new())
    }

    pub fn enabled(&self) -> Vec<Arc<dyn WorkSource>> {
        self.entries.values().map(|e| e.source.clone()).collect()
    }

    pub fn list(&self) -> Vec<SourceSummary> {
        SourceId::ALL
            .iter()
            .map(|id| SourceSummary {
                source: *id,
                enabled: self.entries.contains_key(id),
                site: self.entries.get(id).map(|e| e.source.site()),
            })
            .collect()
    }

    fn entry(&self, id: SourceId) -> Result<&Entry> {
        self.entries
            .get(&id)
            .ok_or_else(|| CorvusError::Conflict(format!("{id} is not configured")))
    }

    fn author(id: SourceId) -> AgentName {
        AgentName::parse(id.as_str()).expect("source ids are valid agent names")
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Services) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let s = self.services.clone();
        tokio::task::spawn_blocking(move || f(&s))
            .await
            .map_err(|e| CorvusError::Internal(format!("blocking task failed: {e}")))?
    }

    /// Takes the cross-process lock for a pass (blocks until any other process's pass ends).
    async fn file_lock(&self, id: SourceId) -> Result<std::fs::File> {
        let dir = self.state_dir.join("run");
        tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
            crate::config::ensure_private_dir(&dir)
                .map_err(|e| CorvusError::Internal(format!("{}: {e}", dir.display())))?;
            let path = dir.join(format!("source-{id}.lock"));
            let f = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .map_err(|e| CorvusError::Internal(format!("{}: {e}", path.display())))?;
            f.lock()
                .map_err(|e| CorvusError::Internal(format!("{}: {e}", path.display())))?;
            Ok(f)
        })
        .await
        .map_err(|e| CorvusError::Internal(format!("lock task failed: {e}")))?
    }

    async fn filter_of(&self, id: SourceId, default: SourceFilter) -> Result<SourceFilter> {
        let stored = self
            .blocking(move |s| s.db().read(|tx| store::get_filter(tx, id.as_str())))
            .await?;
        Ok(stored
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or(default))
    }

    /// One sync pass now (waits for a running pass first). A failed pass is reported in the result.
    pub async fn sync(&self, id: SourceId) -> Result<SyncResult> {
        let entry = self.entry(id)?;
        let _flight = entry.flight.lock().await;
        let _file = self.file_lock(id).await?;
        let now = self.services.clock().now_ms();
        let filter = self.filter_of(id, entry.source.default_filter()).await?;
        match self.fetch(entry, &filter, now).await {
            Ok((tickets, comments)) => {
                let src = entry.source.clone();
                let stats = self
                    .blocking(move |s| reconcile(s, src.as_ref(), &filter, &tickets, &comments, now))
                    .await;
                match stats {
                    Ok(stats) => Ok(SyncResult {
                        ok: true,
                        stats: Some(stats),
                        error: None,
                    }),
                    Err(e) => self.fail(id, e, now).await,
                }
            }
            Err(e) => self.fail(id, e, now).await,
        }
    }

    async fn fail(&self, id: SourceId, e: CorvusError, now: i64) -> Result<SyncResult> {
        let msg = e.to_string();
        tracing::warn!(source = id.as_str(), error = %msg, "source sync failed; nothing written");
        let m = msg.clone();
        self.blocking(move |s| {
            s.db().write(|tx| {
                store::record_error(tx, id.as_str(), &m)?;
                events::insert(
                    tx,
                    EventKind::SourceSyncFailed,
                    &NewEvent {
                        summary: format!("{} sync failed", id.title()),
                        agent_name: Some(id.as_str()),
                        ..NewEvent::default()
                    },
                    now,
                )?;
                Ok(())
            })
        })
        .await?;
        Ok(SyncResult {
            ok: false,
            stats: None,
            error: Some(msg),
        })
    }

    /// All network reads, before any write: the search, then comments for every ticket that will be acted on.
    async fn fetch(
        &self,
        entry: &Entry,
        filter: &SourceFilter,
        now: i64,
    ) -> Result<(Vec<Ticket>, HashMap<String, Vec<Comment>>)> {
        let id = entry.source.id();
        let tickets = entry.source.search(filter, now).await?;
        let links: HashMap<String, LinkRow> = self
            .blocking(move |s| s.db().read(|tx| store::live_links(tx, id.as_str())))
            .await?
            .into_iter()
            .map(|l| (l.ticket_key.clone(), l))
            .collect();
        let mut comments = HashMap::new();
        for t in &tickets {
            if needs_comments(filter, t, links.get(t.key.as_str())) {
                comments.insert(t.key.to_string(), entry.source.comments(&t.key).await?);
            }
        }
        Ok((tickets, comments))
    }

    /// The board: snapshot, links, filter, statuses and sync state.
    pub async fn view(&self, id: SourceId) -> Result<SourceView> {
        let Some(entry) = self.entries.get(&id) else {
            return Ok(SourceView {
                source: id,
                enabled: false,
                site: None,
                filter: None,
                tickets: Vec::new(),
                known_statuses: Vec::new(),
                last_sync_at: None,
                last_stats: None,
                last_error: None,
            });
        };
        let filter = self.filter_of(id, entry.source.default_filter()).await?;
        let known = self.known_statuses(entry).await;
        let f = filter.clone();
        let (rows, links, state) = self
            .blocking(move |s| {
                s.db().read(|tx| {
                    Ok((
                        store::tickets(tx, id.as_str())?,
                        store::live_links(tx, id.as_str())?,
                        store::sync_state(tx, id.as_str())?,
                    ))
                })
            })
            .await?;
        let links: HashMap<String, LinkRow> = links.into_iter().map(|l| (l.ticket_key.clone(), l)).collect();
        let tickets = rows
            .into_iter()
            .map(|r| {
                let link = links.get(&r.key);
                let category = StatusCategory::parse(&r.status_category).unwrap_or(StatusCategory::Todo);
                let as_ticket = row_as_ticket(&r, category);
                SourceTicketView {
                    auto_queue: as_ticket.as_ref().is_some_and(|t| f.auto_queues(t)),
                    manual: link.is_some_and(|l| l.manual),
                    task_id: link.map(|l| l.task_id.clone()),
                    task_status: link.and_then(|l| l.task_status.parse().ok()),
                    url: if r.url.starts_with("https://") {
                        r.url
                    } else {
                        String::new()
                    },
                    key: r.key,
                    project: r.project,
                    summary: r.summary,
                    status: r.status,
                    status_category: category,
                    priority: r.priority,
                    resolved_at: r.resolved_at.map(Timestamp),
                    assignee: r.assignee,
                }
            })
            .collect();
        Ok(SourceView {
            source: id,
            enabled: true,
            site: Some(entry.source.site()),
            filter: Some(filter),
            tickets,
            known_statuses: known,
            last_sync_at: state.last_sync_at.map(Timestamp),
            last_stats: state.last_stats.and_then(|j| serde_json::from_str(&j).ok()),
            last_error: state.last_error,
        })
    }

    async fn known_statuses(&self, entry: &Entry) -> Vec<String> {
        {
            let cache = entry.statuses.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, v)) = cache.as_ref()
                && at.elapsed() < STATUS_CACHE
            {
                return v.clone();
            }
        }
        match entry.source.known_statuses().await {
            Ok(mut v) => {
                v.sort();
                v.dedup();
                *entry.statuses.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((Instant::now(), v.clone()));
                v
            }
            Err(e) => {
                tracing::warn!(source = entry.source.id().as_str(), error = %e, "could not fetch statuses");
                Vec::new()
            }
        }
    }

    /// Validates and saves the filter, then runs a pass. The save stands even if the pass fails.
    pub async fn save_filter(&self, id: SourceId, filter: SourceFilter) -> Result<SourceView> {
        self.entry(id)?;
        filter.validate()?;
        let json = serde_json::to_string(&filter).map_err(|e| CorvusError::Internal(e.to_string()))?;
        let now = self.services.clock().now_ms();
        self.blocking(move |s| s.db().write(|tx| store::put_filter(tx, id.as_str(), &json, now)))
            .await?;
        let _ = self.sync(id).await?;
        self.view(id).await
    }

    /// Queues a ticket by hand: well-formed, on the board, not done, and no live task for it.
    pub async fn queue(&self, id: SourceId, req: &QueueTicket) -> Result<TaskView> {
        let entry = self.entry(id)?;
        let key = TicketKey::parse(req.key.trim())?;
        let _flight = entry.flight.lock().await;
        let _file = self.file_lock(id).await?;
        let k = key.to_string();
        let on_board = self
            .blocking(move |s| {
                s.db()
                    .read(|tx| Ok(store::tickets(tx, id.as_str())?.into_iter().any(|t| t.key == k)))
            })
            .await?;
        if !on_board {
            return Err(CorvusError::NotFound(format!(
                "{key} is not on the {} board",
                id.title()
            )));
        }
        // Fresh data for the task body (network, before any write).
        let now = self.services.clock().now_ms();
        let filter = self.filter_of(id, entry.source.default_filter()).await?;
        let ticket = entry
            .source
            .search(&filter, now)
            .await?
            .into_iter()
            .find(|t| t.key == key)
            .ok_or_else(|| {
                CorvusError::NotFound(format!("{key} is no longer on the {} board", id.title()))
            })?;
        if ticket.status_category == StatusCategory::Done {
            return Err(CorvusError::Conflict(format!("{key} is done")));
        }
        let comments = entry.source.comments(&key).await?;
        let src = entry.source.clone();
        self.blocking(move |s| {
            let task_id = s.db().write(|tx| {
                if let Some(l) = store::live_link(tx, id.as_str(), key.as_str())?
                    && live(&l.task_status)
                {
                    return Err(CorvusError::Conflict(format!(
                        "{key} already has a live task ({})",
                        l.task_id
                    )));
                }
                create_for_ticket(tx, src.as_ref(), &ticket, &comments, true, now).map(|(t, _)| t)
            })?;
            s.huginn().get_task(&task_id)
        })
        .await
    }
}

fn live(task_status: &str) -> bool {
    task_status == TaskStatus::Open.as_str() || task_status == TaskStatus::Claimed.as_str()
}

fn row_as_ticket(r: &TicketRow, category: StatusCategory) -> Option<Ticket> {
    Some(Ticket {
        key: TicketKey::parse(&r.key).ok()?,
        project: r.project.clone(),
        summary: r.summary.clone(),
        status: r.status.clone(),
        status_category: category,
        priority: r.priority,
        url: r.url.clone(),
        updated_stamp: r.updated_stamp.clone(),
        resolved_at: r.resolved_at,
        assignee: r.assignee.clone(),
        description: String::new(),
    })
}

/// Whether the pass must fetch this ticket's comments.
fn needs_comments(filter: &SourceFilter, t: &Ticket, link: Option<&LinkRow>) -> bool {
    let aq = filter.auto_queues(t);
    match link {
        None => aq,
        Some(l) => {
            let task_live = live(&l.task_status);
            if l.manual && (!task_live || t.status_category == StatusCategory::Done) {
                return false;
            }
            let tracked = l.manual || l.in_scope || (aq && task_live);
            if tracked && l.updated_stamp != t.updated_stamp {
                return true;
            }
            // Re-entering scope after the task finished: a fresh task needs the comments for its body.
            !task_live && !l.in_scope && !l.manual && aq
        }
    }
}

/// Creates a task and its link for a ticket; all current comments count as processed (the body shows the last
/// five). Returns (task id, created).
fn create_for_ticket(
    tx: &WriteTx<'_>,
    source: &dyn WorkSource,
    t: &Ticket,
    comments: &[Comment],
    manual: bool,
    now: i64,
) -> Result<(String, bool)> {
    let id = source.id();
    let ignored = source.ignored_authors();
    let shown: Vec<&Comment> = comments
        .iter()
        .filter(|c| !ignored.contains(&c.author_id))
        .collect();
    let title = task_title(t);
    let body = task_body(id, t, &shown);
    // `<source>:<KEY>:<stamp>` for a ticket's first task; later fresh tasks (re-entry, re-queue after the task
    // finished) add the link count, so an unchanged stamp cannot collide with a finished task's key.
    let previous = store::link_count(tx, id.as_str(), t.key.as_str())?;
    let key = if previous == 0 {
        format!("{id}:{}:{}", t.key, t.updated_stamp)
    } else {
        format!("{id}:{}:{}:{previous}", t.key, t.updated_stamp)
    };
    let (task_id, created) = create_task_tx(
        tx,
        &Sources::author(id),
        &title,
        &body,
        t.priority,
        &source.pool(),
        Some(&key),
        now,
    )?;
    let all: Vec<&Comment> = comments.iter().collect();
    let (hwm, ids) = advance_mark(0, &BTreeSet::new(), &all);
    if created {
        store::insert_link(
            tx,
            id.as_str(),
            t.key.as_str(),
            &task_id,
            &t.updated_stamp,
            hwm,
            &ids,
            manual,
            now,
        )?;
        events::insert(
            tx,
            EventKind::SourceTaskCreated,
            &NewEvent {
                summary: format!(
                    "{} {} queued{}",
                    id.title(),
                    t.key,
                    if manual { " by hand" } else { "" }
                ),
                agent_name: Some(id.as_str()),
                task_id: Some(&task_id),
                ..NewEvent::default()
            },
            now,
        )?;
    }
    Ok((task_id, created))
}

/// The database half of a pass. Each step is its own short write transaction.
fn reconcile(
    s: &Services,
    source: &dyn WorkSource,
    filter: &SourceFilter,
    tickets: &[Ticket],
    comments: &HashMap<String, Vec<Comment>>,
    now: i64,
) -> Result<SyncStats> {
    let id = source.id();
    let mut stats = SyncStats {
        tickets: tickets.len(),
        ..SyncStats::default()
    };
    let rows: Vec<TicketRow> = tickets
        .iter()
        .map(|t| TicketRow {
            key: t.key.to_string(),
            project: t.project.clone(),
            summary: t.summary.clone(),
            status: t.status.clone(),
            status_category: t.status_category.as_str().to_owned(),
            priority: t.priority,
            url: t.url.clone(),
            updated_stamp: t.updated_stamp.clone(),
            resolved_at: t.resolved_at,
            assignee: t.assignee.clone(),
        })
        .collect();
    s.db()
        .write(|tx| store::replace_tickets(tx, id.as_str(), &rows))?;

    let ignored = source.ignored_authors();
    let empty = Vec::new();
    for t in tickets {
        let cs = comments.get(t.key.as_str()).unwrap_or(&empty);
        let delta = s
            .db()
            .write(|tx| reconcile_ticket(tx, source, filter, t, cs, &ignored, now))?;
        stats.tasks_created += delta.tasks_created;
        stats.comments_delivered += delta.comments_delivered;
        stats.followups += delta.followups;
        stats.resumed += delta.resumed;
        stats.out_of_scope += delta.out_of_scope;
    }
    // Links whose ticket left the result set entirely are out of scope too (manual links are exempt).
    let seen: BTreeSet<String> = tickets.iter().map(|t| t.key.to_string()).collect();
    let gone = s.db().write(|tx| {
        let mut n = 0;
        for l in store::live_links(tx, id.as_str())? {
            if !seen.contains(&l.ticket_key) && l.in_scope && !l.manual {
                store::set_in_scope(tx, l.id, false)?;
                scope_event(tx, id, &l.ticket_key, &l.task_id, now)?;
                n += 1;
            }
        }
        Ok(n)
    })?;
    stats.out_of_scope += gone;
    let json = serde_json::to_string(&stats).unwrap_or_default();
    s.db()
        .write(|tx| store::record_success(tx, id.as_str(), now, &json))?;
    Ok(stats)
}

fn scope_event(tx: &WriteTx<'_>, id: SourceId, key: &str, task_id: &str, now: i64) -> Result<()> {
    events::insert(
        tx,
        EventKind::SourceOutOfScope,
        &NewEvent {
            summary: format!(
                "{} {key} left the auto-queue rules; its task no longer follows the ticket",
                id.title()
            ),
            agent_name: Some(id.as_str()),
            task_id: Some(task_id),
            ..NewEvent::default()
        },
        now,
    )
    .map(|_| ())
}

/// One ticket, in one write transaction that reads the task's status and branches on it.
fn reconcile_ticket(
    tx: &WriteTx<'_>,
    source: &dyn WorkSource,
    filter: &SourceFilter,
    t: &Ticket,
    comments: &[Comment],
    ignored: &[String],
    now: i64,
) -> Result<SyncStats> {
    let id = source.id();
    let mut st = SyncStats::default();
    let aq = filter.auto_queues(t);
    let link = store::live_link(tx, id.as_str(), t.key.as_str())?;
    let Some(l) = link else {
        if aq && create_for_ticket(tx, source, t, comments, false, now)?.1 {
            st.tasks_created += 1;
        }
        return Ok(st);
    };
    let task_live = live(&l.task_status);

    // Re-entering scope after the task finished → a fresh task and link.
    if !task_live && !l.in_scope && !l.manual {
        if aq && create_for_ticket(tx, source, t, comments, false, now)?.1 {
            st.tasks_created += 1;
        }
        return Ok(st);
    }
    // Manual links follow comments only while their task is live and the ticket is not done (system/07 step 6).
    if l.manual && (!task_live || t.status_category == StatusCategory::Done) {
        return Ok(st);
    }
    // Out of scope with a live task, back in scope → resume.
    if !l.in_scope && !l.manual {
        if !(aq && task_live) {
            return Ok(st); // out of scope: comments are not followed
        }
        store::set_in_scope(tx, l.id, true)?;
        st.resumed += 1;
    }

    if l.updated_stamp != t.updated_stamp {
        let new: Vec<&Comment> = comments
            .iter()
            .filter(|c| is_new_comment(c, l.comment_hwm, &l.hwm_ids))
            .collect();
        let deliver: Vec<&Comment> = new
            .iter()
            .copied()
            .filter(|c| !ignored.contains(&c.author_id))
            .collect();
        let (hwm, ids) = advance_mark(l.comment_hwm, &l.hwm_ids, &new);
        if deliver.is_empty() {
            store::update_link_mark(tx, l.id, &t.updated_stamp, hwm, &ids)?;
        } else if l.task_status == TaskStatus::Open.as_str() {
            append_tx(tx, &l.task_id, &comments_text(id, &t.key, &deliver), now)?;
            store::update_link_mark(tx, l.id, &t.updated_stamp, hwm, &ids)?;
            routed_event(
                tx,
                id,
                t,
                &l.task_id,
                deliver.len(),
                "appended to the open task",
                None,
                now,
            )?;
            st.comments_delivered += deliver.len();
        } else if l.task_status == TaskStatus::Claimed.as_str() {
            let claim = claims::active_for_task(tx, &l.task_id)?
                .ok_or_else(|| CorvusError::Internal("claimed task without a claim".into()))?;
            let thread = threads::for_task(tx, &l.task_id)?
                .ok_or_else(|| CorvusError::Internal("task without a thread".into()))?;
            let to = AgentName::parse(&claim.agent_name)?;
            let body = comments_text(id, &t.key, &deliver);
            let msg = post_in_tx(
                tx,
                &thread,
                &Sources::author(id),
                Some(&to),
                MessageType::Note,
                None,
                &body,
                &[],
                None,
                None,
                now,
            )?;
            store::update_link_mark(tx, l.id, &t.updated_stamp, hwm, &ids)?;
            routed_event(
                tx,
                id,
                t,
                &l.task_id,
                deliver.len(),
                &format!("sent to {to}"),
                Some(msg.id),
                now,
            )?;
            st.comments_delivered += deliver.len();
        } else {
            // Done or cancelled: a follow-up task carries the comments and becomes the live link.
            let title = format!("[{}] Follow-up: {}", t.key, clip(&t.summary));
            let body = followup_body(id, t, &deliver);
            // The comment ids at the new mark make the key tie-safe (D0012): a later comment with the same order
            // key gets its own follow-up instead of colliding with the previous one.
            let key = format!("{id}:followup:{}:{hwm}:{}", t.key, ids_digest(&ids));
            let (task_id, created) = create_task_tx(
                tx,
                &Sources::author(id),
                &title,
                &body,
                t.priority,
                &source.pool(),
                Some(&key),
                now,
            )?;
            if created {
                store::update_link_mark(tx, l.id, &t.updated_stamp, hwm, &ids)?;
                store::insert_link(
                    tx,
                    id.as_str(),
                    t.key.as_str(),
                    &task_id,
                    &t.updated_stamp,
                    hwm,
                    &ids,
                    l.manual,
                    now,
                )?;
                routed_event(
                    tx,
                    id,
                    t,
                    &task_id,
                    deliver.len(),
                    "carried by a follow-up task",
                    None,
                    now,
                )?;
                st.followups += 1;
                st.comments_delivered += deliver.len();
            }
        }
    }

    // Leaving scope (manual links are exempt).
    if !aq && !l.manual && (l.in_scope || st.resumed > 0) {
        store::set_in_scope(tx, l.id, false)?;
        scope_event(tx, id, t.key.as_str(), &l.task_id, now)?;
        st.out_of_scope += 1;
    }
    Ok(st)
}

/// A short, stable digest of a set of comment ids.
fn ids_digest(ids: &BTreeSet<String>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for id in ids {
        h.update(id.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[allow(clippy::too_many_arguments)]
fn routed_event(
    tx: &WriteTx<'_>,
    id: SourceId,
    t: &Ticket,
    task_id: &str,
    n: usize,
    how: &str,
    message_id: Option<i64>,
    now: i64,
) -> Result<()> {
    events::insert(
        tx,
        EventKind::SourceCommentRouted,
        &NewEvent {
            summary: format!("{n} new {} comment(s) on {}: {how}", id.title(), t.key),
            agent_name: Some(id.as_str()),
            task_id: Some(task_id),
            message_id,
            ..NewEvent::default()
        },
        now,
    )
    .map(|_| ())
}
