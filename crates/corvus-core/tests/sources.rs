//! Plan 0008 "Engine" verification: the work-source engine against a scripted in-memory fake source.
//!
//! Every assertion is on database state (tasks, messages, events, links) or on the engine's returned values; the
//! fake source knows nothing about Jira or Linear (`id()` is the only difference between the two).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvus_core::Services;
use corvus_core::api::{
    CancelTask, ClaimTask, CompleteTask, CreateTask, ListMessages, ListTasks, MessageListItem, QueueTicket,
    ReadInbox, SourceTicketView, SyncStats, TaskView,
};
use corvus_core::background::{LoopTimings, STEPS, run_background_with_sources};
use corvus_core::sources::{BoxFuture, Sources, WorkSource};
use corvus_core::testing::{TestDb, count, query_i64};
use corvus_domain::sources::untrusted_notice;
use corvus_domain::{
    AutoQueueRule, Comment, CorvusError, Result, SourceFilter, SourceId, StatusCategory, TaskStatus, Ticket,
    TicketKey,
};
use tokio::sync::{Notify, watch};

// ---------------------------------------------------------------------------------------------------------
// The fake source
// ---------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Search,
    Comments,
}

#[derive(Debug, Default)]
struct State {
    tickets: Vec<Ticket>,
    comments: HashMap<String, Vec<Comment>>,
    fail_search: bool,
    fail_comments: HashSet<String>,
    /// Fail the Nth call (search + comment fetches, counted from the fake's creation).
    fail_on_call: Option<usize>,
    calls: usize,
    ignored: Vec<String>,
    hold: Option<(Hold, Arc<Notify>)>,
    filter: SourceFilter,
}

#[derive(Debug)]
struct Fake {
    id: SourceId,
    pool: String,
    interval: Duration,
    st: Mutex<State>,
    log: Arc<Mutex<Vec<String>>>,
    /// Signalled (one permit) when a held call has started.
    entered: Notify,
    search_calls: AtomicUsize,
}

fn default_filter() -> SourceFilter {
    SourceFilter {
        statuses: vec!["To Do".into(), "In Progress".into()],
        done_days: 7,
        projects: vec![],
        auto_queue: vec![AutoQueueRule {
            projects: vec!["AA".into()],
            statuses: vec!["To Do".into()],
        }],
    }
}

impl Fake {
    fn new(id: SourceId) -> Arc<Self> {
        Self::with_log(id, Arc::default())
    }

    fn with_log(id: SourceId, log: Arc<Mutex<Vec<String>>>) -> Arc<Self> {
        Arc::new(Self {
            id,
            pool: "ext".into(),
            interval: Duration::from_millis(80),
            st: Mutex::new(State {
                filter: default_filter(),
                ..State::default()
            }),
            log,
            entered: Notify::new(),
            search_calls: AtomicUsize::new(0),
        })
    }

    fn set(&self, tickets: Vec<Ticket>) {
        self.st.lock().unwrap().tickets = tickets;
    }

    fn upsert(&self, t: Ticket) {
        let mut st = self.st.lock().unwrap();
        st.tickets.retain(|x| x.key != t.key);
        st.tickets.push(t);
    }

    fn remove(&self, key: &str) {
        self.st.lock().unwrap().tickets.retain(|x| x.key.as_str() != key);
    }

    fn add_comment(&self, key: &str, c: Comment) {
        self.st
            .lock()
            .unwrap()
            .comments
            .entry(key.into())
            .or_default()
            .push(c);
    }

    fn set_search_failing(&self, on: bool) {
        self.st.lock().unwrap().fail_search = on;
    }

    fn set_comments_failing(&self, key: &str, on: bool) {
        let mut st = self.st.lock().unwrap();
        if on {
            st.fail_comments.insert(key.into());
        } else {
            st.fail_comments.remove(key);
        }
    }

    /// Fail the `n`th call from now (1 = the very next one).
    fn fail_in(&self, n: usize) {
        let mut st = self.st.lock().unwrap();
        st.fail_on_call = Some(st.calls + n);
    }

    fn set_ignored(&self, ids: &[&str]) {
        self.st.lock().unwrap().ignored = ids.iter().map(|s| (*s).to_owned()).collect();
    }

    fn hold(&self, what: Hold) {
        self.st.lock().unwrap().hold = Some((what, Arc::new(Notify::new())));
    }

    /// Lets every held (and later) call through.
    fn release(&self) {
        let h = self.st.lock().unwrap().hold.take();
        if let Some((_, gate)) = h {
            gate.notify_one();
        }
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.entered.notified())
            .await
            .expect("the held call never started");
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn calls(&self, prefix: &str) -> usize {
        self.log()
            .iter()
            .filter(|l| l.starts_with(prefix) && l.ends_with(":start"))
            .count()
    }

    fn begin(&self, kind: Hold, key: Option<&str>) -> (bool, Option<Arc<Notify>>) {
        let label = key.map_or_else(|| "search".to_owned(), |k| format!("comments:{k}"));
        let mut st = self.st.lock().unwrap();
        st.calls += 1;
        self.log.lock().unwrap().push(format!("{label}:start"));
        let mut fail = st.fail_on_call == Some(st.calls);
        fail |= match kind {
            Hold::Search => st.fail_search,
            Hold::Comments => key.is_some_and(|k| st.fail_comments.contains(k)),
        };
        let gate = st
            .hold
            .as_ref()
            .filter(|(h, _)| *h == kind)
            .map(|(_, g)| g.clone());
        if gate.is_some() {
            self.entered.notify_one();
        }
        (fail, gate)
    }
}

impl WorkSource for Fake {
    fn id(&self) -> SourceId {
        self.id
    }
    fn site(&self) -> String {
        "fake.example".into()
    }
    fn default_filter(&self) -> SourceFilter {
        self.st.lock().unwrap().filter.clone()
    }
    fn pool(&self) -> String {
        self.pool.clone()
    }
    fn poll_interval(&self) -> Duration {
        self.interval
    }
    fn ignored_authors(&self) -> Vec<String> {
        self.st.lock().unwrap().ignored.clone()
    }
    fn search<'a>(&'a self, _filter: &'a SourceFilter, _now_ms: i64) -> BoxFuture<'a, Result<Vec<Ticket>>> {
        Box::pin(async move {
            self.search_calls.fetch_add(1, Ordering::SeqCst);
            let (fail, gate) = self.begin(Hold::Search, None);
            if let Some(g) = gate {
                g.notified().await;
            }
            self.log.lock().unwrap().push("search:end".into());
            if fail {
                return Err(CorvusError::Internal("injected search failure".into()));
            }
            Ok(self.st.lock().unwrap().tickets.clone())
        })
    }
    fn comments<'a>(&'a self, key: &'a TicketKey) -> BoxFuture<'a, Result<Vec<Comment>>> {
        Box::pin(async move {
            let (fail, gate) = self.begin(Hold::Comments, Some(key.as_str()));
            if let Some(g) = gate {
                g.notified().await;
            }
            self.log.lock().unwrap().push(format!("comments:{key}:end"));
            if fail {
                return Err(CorvusError::Internal(format!(
                    "injected comment failure for {key}"
                )));
            }
            Ok(self
                .st
                .lock()
                .unwrap()
                .comments
                .get(key.as_str())
                .cloned()
                .unwrap_or_default())
        })
    }
    fn known_statuses(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async { Ok(vec!["To Do".to_owned(), "In Progress".to_owned()]) })
    }
}

// ---------------------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------------------

fn tk(key: &str, status: &str, stamp: &str) -> Ticket {
    let k = TicketKey::parse(key).unwrap();
    Ticket {
        project: k.project().to_owned(),
        summary: format!("Summary of {key}"),
        status: status.into(),
        status_category: if status == "Done" {
            StatusCategory::Done
        } else if status == "In Progress" {
            StatusCategory::InProgress
        } else {
            StatusCategory::Todo
        },
        priority: 3,
        url: format!("https://tracker.example/browse/{key}"),
        updated_stamp: stamp.into(),
        resolved_at: (status == "Done").then_some(1_000),
        assignee: Some("Ada".into()),
        description: format!("Steps to reproduce {key}"),
        key: k,
    }
}

fn cm(id: &str, order: i64, author: &str, body: &str) -> Comment {
    Comment {
        id: id.into(),
        order_key: order,
        author_id: author.into(),
        author_name: format!("Name of {author}"),
        created_at: order,
        body: body.into(),
    }
}

struct H {
    t: TestDb,
    svc: Services,
    fake: Arc<Fake>,
    src: Arc<Sources>,
    id: SourceId,
}

fn h(id: SourceId) -> H {
    let t = TestDb::new();
    let svc = t.services();
    let fake = Fake::new(id);
    let src = Arc::new(Sources::new(
        svc.clone(),
        t.dir.path().join("state"),
        vec![fake.clone() as Arc<dyn WorkSource>],
    ));
    H {
        t,
        svc,
        fake,
        src,
        id,
    }
}

async fn within<F: Future>(f: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), f)
        .await
        .expect("timed out")
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

impl H {
    /// A pass that must succeed.
    async fn sync(&self) -> SyncStats {
        let r = within(self.src.sync(self.id)).await.unwrap();
        assert!(r.ok, "pass failed: {:?}", r.error);
        r.stats.expect("stats on success")
    }

    async fn ticket(&self, key: &str) -> Option<SourceTicketView> {
        within(self.src.view(self.id))
            .await
            .unwrap()
            .tickets
            .into_iter()
            .find(|t| t.key == key)
    }

    async fn task_id(&self, key: &str) -> String {
        self.ticket(key).await.unwrap().task_id.expect("a live link")
    }

    fn task(&self, id: &str) -> TaskView {
        self.svc.huginn().get_task(id).unwrap()
    }

    fn all_tasks(&self) -> Vec<TaskView> {
        self.svc
            .huginn()
            .list_tasks(&ListTasks {
                limit: Some(200),
                ..ListTasks::default()
            })
            .unwrap()
    }

    fn thread_messages(&self, task: &TaskView) -> Vec<MessageListItem> {
        self.svc
            .muninn()
            .list_messages(&ListMessages {
                thread: Some(task.thread_id.clone().expect("task thread")),
                limit: Some(100),
                ..ListMessages::default()
            })
            .unwrap()
    }

    fn claim(&self, agent: &str) -> (String, TaskView) {
        let sid = self.t.session(agent).to_string();
        let r = self
            .svc
            .huginn()
            .claim_task(&ClaimTask {
                session_id: sid.clone(),
                pool: None,
            })
            .unwrap();
        (sid, r.task.expect("something claimable"))
    }

    fn finish(&self, agent: &str) -> String {
        let (sid, task) = self.claim(agent);
        self.svc
            .huginn()
            .complete_task(
                &task.id,
                &CompleteTask {
                    session_id: sid,
                    result_summary: Some("done".into()),
                },
            )
            .unwrap();
        task.id
    }

    fn cancel(&self, task: &str) {
        self.svc
            .huginn()
            .cancel_task(
                task,
                &CancelTask {
                    by: "op".into(),
                    ..CancelTask::default()
                },
            )
            .unwrap();
    }

    fn n(&self, table: &str) -> i64 {
        count(&self.t.db, table)
    }

    /// Events other than the failure audit entry.
    fn work_events(&self) -> i64 {
        query_i64(
            &self.t.db,
            "SELECT count(*) FROM events WHERE kind != 'source.sync_failed'",
        )
        .unwrap()
    }

    fn in_scope(&self, key: &str) -> Vec<i64> {
        let sql =
            format!("SELECT group_concat(in_scope) FROM source_links WHERE ticket_key = '{key}' ORDER BY id");
        self.svc
            .db()
            .read(|tx| {
                use corvus_core::db::Reader;
                let s: Option<String> = Reader::sql(tx)
                    .query_row(&sql, [], |r| r.get(0))
                    .map_err(|e| CorvusError::Internal(e.to_string()))?;
                Ok(s.unwrap_or_default()
                    .split(',')
                    .filter(|x| !x.is_empty())
                    .map(|x| x.parse().unwrap())
                    .collect())
            })
            .unwrap()
    }

    /// (tasks, messages, events, links, snapshot rows)
    fn counts(&self) -> (i64, i64, i64, i64, i64) {
        (
            self.n("tasks"),
            self.n("messages"),
            self.n("events"),
            self.n("source_links"),
            self.n("source_tickets"),
        )
    }
}

fn occurrences(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// ---------------------------------------------------------------------------------------------------------
// Auto-queueing and task content
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn assigned_ticket_matching_a_rule_creates_exactly_one_task() {
    for id in SourceId::ALL {
        let h = h(id);
        h.fake
            .set(vec![tk("AA-1", "To Do", "s1"), tk("BB-2", "To Do", "s1")]);
        h.fake.add_comment("AA-1", cm("c1", 10, "u1", "first remark"));
        h.fake.add_comment("AA-1", cm("c2", 20, "u2", "second remark"));

        let st = h.sync().await;
        assert_eq!(st.tickets, 2);
        assert_eq!(st.tasks_created, 1, "{id}: only AA-1 is covered by a rule");
        assert_eq!(h.n("tasks"), 1);

        let tid = h.task_id("AA-1").await;
        let task = h.task(&tid);
        assert_eq!(task.title, "[AA-1] Summary of AA-1");
        assert_eq!(task.priority, 3);
        assert_eq!(task.pool, "ext");
        assert_eq!(task.creator, id.as_str(), "author agent is the source id");
        assert_eq!(task.status, TaskStatus::Open);
        let b = &task.body;
        assert!(b.contains("https://tracker.example/browse/AA-1"), "{b}");
        assert!(b.contains(&untrusted_notice(id)), "{b}");
        assert!(
            b.contains(&format!("UNTRUSTED text copied from {}", id.title())),
            "{b}"
        );
        let notice_at = b.find("UNTRUSTED").unwrap();
        assert!(b.find("Steps to reproduce AA-1").unwrap() > notice_at);
        assert!(b.contains("first remark") && b.contains("second remark"), "{b}");
        assert!(b.find("first remark").unwrap() > notice_at);

        // The link is visible on the board; the uncovered ticket is on the board but has no task.
        let v = h.ticket("AA-1").await.unwrap();
        assert!(v.auto_queue && !v.manual);
        assert_eq!(v.task_status, Some(TaskStatus::Open));
        let other = h.ticket("BB-2").await.unwrap();
        assert!(!other.auto_queue && other.task_id.is_none());

        assert_eq!(
            query_i64(
                &h.t.db,
                "SELECT count(*) FROM events WHERE kind = 'source.task_created'"
            ),
            Some(1)
        );
        // Comments present at creation are folded into the body, not delivered again later.
        let again = h.sync().await;
        assert_eq!((again.tasks_created, again.comments_delivered), (0, 0));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_repoll_writes_nothing() {
    for id in SourceId::ALL {
        let h = h(id);
        h.fake
            .set(vec![tk("AA-1", "To Do", "s1"), tk("AA-2", "In Progress", "s1")]);
        h.fake.add_comment("AA-1", cm("c1", 10, "u1", "hello"));
        h.sync().await;
        let before = h.counts();
        let tasks_before: Vec<(String, String)> = h.all_tasks().into_iter().map(|t| (t.id, t.body)).collect();
        let comment_calls = h.fake.calls("comments:");

        let st = h.sync().await;
        assert_eq!(
            (
                st.tasks_created,
                st.comments_delivered,
                st.followups,
                st.resumed,
                st.out_of_scope
            ),
            (0, 0, 0, 0, 0)
        );
        assert_eq!(st.tickets, 2);
        assert_eq!(h.counts(), before, "{id}: no tasks, messages, events or links");
        let tasks_after: Vec<(String, String)> = h.all_tasks().into_iter().map(|t| (t.id, t.body)).collect();
        assert_eq!(tasks_after, tasks_before);
        assert_eq!(
            h.fake.calls("comments:"),
            comment_calls,
            "unchanged tickets are not re-fetched"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------
// Comment routing
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn comments_on_an_open_task_are_appended_to_the_body() {
    for id in SourceId::ALL {
        let h = h(id);
        h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
        h.fake.add_comment("AA-1", cm("c1", 10, "u1", "old remark"));
        h.sync().await;
        let tid = h.task_id("AA-1").await;
        let msgs_before = h.n("messages");

        h.fake.upsert(tk("AA-1", "To Do", "s2"));
        h.fake
            .add_comment("AA-1", cm("c2", 20, "u2", "please also handle X"));
        let st = h.sync().await;
        assert_eq!((st.comments_delivered, st.tasks_created, st.followups), (1, 0, 0));
        let body = h.task(&tid).body;
        assert!(body.contains("please also handle X"), "{body}");
        assert!(body.contains(&untrusted_notice(id)));
        assert_eq!(occurrences(&body, "old remark"), 1, "old comment is not repeated");
        assert_eq!(h.n("tasks"), 1);
        assert_eq!(h.n("messages"), msgs_before, "open tasks get no message");
        assert_eq!(
            query_i64(
                &h.t.db,
                "SELECT count(*) FROM events WHERE kind = 'source.comment_routed'"
            ),
            Some(1)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_on_a_claimed_task_become_one_message_to_the_claimant() {
    for id in SourceId::ALL {
        let h = h(id);
        h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
        h.sync().await;
        let (_sid, task) = h.claim("worker");
        let body_before = h.task(&task.id).body;

        h.fake.upsert(tk("AA-1", "To Do", "s2"));
        h.fake.add_comment("AA-1", cm("c1", 10, "u1", "first note"));
        h.fake.add_comment("AA-1", cm("c2", 20, "u2", "second note"));
        let st = h.sync().await;
        assert_eq!(st.comments_delivered, 2);

        let msgs = h.thread_messages(&task);
        assert_eq!(msgs.len(), 1, "{id}: both comments travel in exactly one message");
        let m = &msgs[0].message;
        assert_eq!(m.sender, id.as_str());
        assert_eq!(m.recipient.as_deref(), Some("worker"));
        assert!(m.body.contains(&untrusted_notice(id)), "{}", m.body);
        assert!(m.body.contains("first note") && m.body.contains("second note"));
        assert!(m.body.find("first note").unwrap() > m.body.find("UNTRUSTED").unwrap());
        assert_eq!(
            h.svc
                .muninn()
                .read_inbox(&ReadInbox {
                    agent: "worker".into(),
                    limit: None,
                    after_id: None
                })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            h.task(&task.id).body,
            body_before,
            "claimed task body is untouched"
        );

        let before = h.counts();
        let again = h.sync().await;
        assert_eq!(again.comments_delivered, 0);
        assert_eq!(h.counts(), before);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_on_a_finished_task_create_a_followup_that_becomes_the_live_link() {
    for how in ["done", "cancelled"] {
        for id in SourceId::ALL {
            let h = h(id);
            h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
            h.sync().await;
            let old = h.task_id("AA-1").await;
            if how == "done" {
                assert_eq!(h.finish("worker"), old);
            } else {
                h.cancel(&old);
            }
            let old_body = h.task(&old).body;

            h.fake.upsert(tk("AA-1", "To Do", "s2"));
            h.fake
                .add_comment("AA-1", cm("c1", 10, "u1", "needs a follow up"));
            let st = h.sync().await;
            assert_eq!(
                (st.followups, st.tasks_created, st.comments_delivered),
                (1, 0, 1),
                "{how}/{id}"
            );
            assert_eq!(h.n("tasks"), 2);

            let v = h.ticket("AA-1").await.unwrap();
            let new = v.task_id.unwrap();
            assert_ne!(new, old, "the follow-up is the live link");
            assert_eq!(v.task_status, Some(TaskStatus::Open));
            let t = h.task(&new);
            assert!(t.title.starts_with("[AA-1] Follow-up"), "{}", t.title);
            assert!(t.body.contains("needs a follow up"));
            assert!(t.body.contains(&untrusted_notice(id)));
            assert_eq!(t.creator, id.as_str());
            assert_eq!(h.task(&old).body, old_body, "the finished task is untouched");
            assert_eq!(
                h.task(&old).status,
                if how == "done" {
                    TaskStatus::Done
                } else {
                    TaskStatus::Cancelled
                }
            );

            // Idempotent, and the next comment follows the new live task (open → appended).
            let before = h.counts();
            h.sync().await;
            assert_eq!(h.counts(), before);
            h.fake.upsert(tk("AA-1", "To Do", "s3"));
            h.fake
                .add_comment("AA-1", cm("c2", 20, "u1", "and a second remark"));
            let st = h.sync().await;
            assert_eq!((st.followups, st.comments_delivered), (0, 1));
            assert!(h.task(&new).body.contains("and a second remark"));
            assert_eq!(h.n("tasks"), 2);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_follow_the_task_state_as_it_changes_between_passes() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let first = h.task_id("AA-1").await;

    // open → appended
    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake
        .add_comment("AA-1", cm("c1", 10, "u1", "remark while open"));
    h.sync().await;
    assert!(h.task(&first).body.contains("remark while open"));
    assert_eq!(h.n("messages"), 0);

    // claimed → message to the claimant
    let (sid, task) = h.claim("worker");
    h.fake.upsert(tk("AA-1", "To Do", "s3"));
    h.fake
        .add_comment("AA-1", cm("c2", 20, "u1", "remark while claimed"));
    h.sync().await;
    let msgs = h.thread_messages(&task);
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].message.body.contains("remark while claimed"));
    assert!(!h.task(&first).body.contains("remark while claimed"));

    // done → follow-up
    h.svc
        .huginn()
        .complete_task(
            &first,
            &CompleteTask {
                session_id: sid,
                result_summary: None,
            },
        )
        .unwrap();
    h.fake.upsert(tk("AA-1", "To Do", "s4"));
    h.fake
        .add_comment("AA-1", cm("c3", 30, "u1", "remark after done"));
    let st = h.sync().await;
    assert_eq!(st.followups, 1);
    let live = h.task_id("AA-1").await;
    assert_ne!(live, first);
    assert!(h.task(&live).body.contains("remark after done"));
    assert_eq!(
        h.thread_messages(&task).len(),
        1,
        "no further message to the old task"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_fetch_leaves_the_mark_unmoved_and_the_next_pass_delivers_once() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let tid = h.task_id("AA-1").await;
    let body_before = h.task(&tid).body;

    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake.add_comment("AA-1", cm("c1", 10, "u1", "important remark"));
    h.fake.set_comments_failing("AA-1", true);
    let before = h.counts();
    let r = within(h.src.sync(h.id)).await.unwrap();
    assert!(!r.ok && r.stats.is_none());
    assert!(r.error.as_deref().unwrap().contains("injected"), "{:?}", r.error);
    assert_eq!(h.task(&tid).body, body_before, "nothing delivered");
    let after = h.counts();
    assert_eq!(
        (after.0, after.1, after.3, after.4),
        (before.0, before.1, before.3, before.4)
    );
    let v = within(h.src.view(h.id)).await.unwrap();
    assert!(v.last_error.as_deref().unwrap().contains("injected"));
    assert_eq!(
        v.tickets.iter().find(|t| t.key == "AA-1").unwrap().summary,
        "Summary of AA-1"
    );

    // The next good pass delivers — exactly once — and clears the error.
    h.fake.set_comments_failing("AA-1", false);
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 1);
    assert_eq!(occurrences(&h.task(&tid).body, "important remark"), 1);
    assert!(within(h.src.view(h.id)).await.unwrap().last_error.is_none());
    let again = h.sync().await;
    assert_eq!(again.comments_delivered, 0);
    assert_eq!(occurrences(&h.task(&tid).body, "important remark"), 1);
}

// ---------------------------------------------------------------------------------------------------------
// Ignored accounts and the order-key tie rule
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ignored_accounts_are_not_delivered_and_do_not_retrigger() {
    let h = h(SourceId::Jira);
    h.fake.set_ignored(&["bot"]);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let tid = h.task_id("AA-1").await;
    let body_before = h.task(&tid).body;

    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake.add_comment("AA-1", cm("b1", 10, "bot", "automated noise"));
    let before = h.counts();
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 0);
    assert_eq!(h.task(&tid).body, body_before);
    assert_eq!(
        h.counts(),
        before,
        "an ignored comment writes no task, message or event"
    );

    // Same stamp again: the mark moved past the ignored comment, so nothing is fetched or delivered.
    let calls = h.fake.calls("comments:");
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 0);
    assert_eq!(h.fake.calls("comments:"), calls);

    // A later pass (stamp changed for another reason) must not deliver the ignored comment either.
    h.fake.upsert(tk("AA-1", "To Do", "s3"));
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 0);
    assert_eq!(h.task(&tid).body, body_before);

    // A mixed batch delivers only the human comment.
    h.fake.upsert(tk("AA-1", "To Do", "s4"));
    h.fake.add_comment("AA-1", cm("h1", 20, "human", "real request"));
    h.fake.add_comment("AA-1", cm("b2", 30, "bot", "more noise"));
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 1);
    let body = h.task(&tid).body;
    assert!(body.contains("real request") && !body.contains("noise"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_with_the_same_order_key_in_different_polls_are_each_delivered_once() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let (_sid, task) = h.claim("worker");

    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake.add_comment("AA-1", cm("a", 100, "u1", "tie-comment-a"));
    assert_eq!(h.sync().await.comments_delivered, 1);

    // `b` arrives later with the same order key as `a`.
    h.fake.upsert(tk("AA-1", "To Do", "s3"));
    h.fake.add_comment("AA-1", cm("b", 100, "u1", "tie-comment-b"));
    assert_eq!(h.sync().await.comments_delivered, 1, "only b is new");

    // And a third at the same key, plus a no-news stamp bump.
    h.fake.upsert(tk("AA-1", "To Do", "s4"));
    h.fake.add_comment("AA-1", cm("c", 100, "u1", "tie-comment-c"));
    assert_eq!(h.sync().await.comments_delivered, 1, "only c is new");
    h.fake.upsert(tk("AA-1", "To Do", "s5"));
    assert_eq!(h.sync().await.comments_delivered, 0);

    let msgs = h.thread_messages(&task);
    assert_eq!(msgs.len(), 3);
    let all: String = msgs.iter().map(|m| m.message.body.clone()).collect();
    for needle in ["tie-comment-a", "tie-comment-b", "tie-comment-c"] {
        assert_eq!(occurrences(&all, needle), 1, "{needle} delivered exactly once");
    }
}

/// BUG (engine): the same tie rule on a finished task. The follow-up's idempotency key is
/// `<source>:followup:<KEY>:<hwm>`; a second batch of comments at the SAME order key reuses that key, so the
/// engine finds the existing follow-up, treats it as "not created", and leaves the mark unmoved — the new comment
/// is never delivered (and is re-fetched on every pass).
#[tokio::test(flavor = "multi_thread")]
async fn tie_comments_on_a_finished_task_are_not_lost() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    h.finish("worker");

    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake.add_comment("AA-1", cm("a", 100, "u1", "tie-a"));
    assert_eq!(h.sync().await.followups, 1);
    let followup = h.task_id("AA-1").await;
    // Finish the follow-up too, so the second tie comment lands on the finished-task path again.
    h.finish("worker2");
    h.fake.upsert(tk("AA-1", "To Do", "s3"));
    h.fake.add_comment("AA-1", cm("b", 100, "u1", "tie-b"));
    h.sync().await;
    let live = h.task_id("AA-1").await;
    assert_ne!(live, followup, "the tie comment b needs a new follow-up");
    assert!(h.task(&live).body.contains("tie-b"));
}

// ---------------------------------------------------------------------------------------------------------
// Re-assignment / scope
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_leaving_the_result_set_goes_out_of_scope_and_resumes_while_the_task_lives() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let tid = h.task_id("AA-1").await;
    assert_eq!(h.in_scope("AA-1"), vec![1]);

    // Re-assigned away.
    h.fake.remove("AA-1");
    let st = h.sync().await;
    assert_eq!(st.out_of_scope, 1);
    assert_eq!(h.in_scope("AA-1"), vec![0]);
    assert!(h.ticket("AA-1").await.is_none(), "no longer on the board");
    assert_eq!(h.n("tasks"), 1);
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM events WHERE kind = 'source.out_of_scope'"
        ),
        Some(1)
    );
    let before = h.counts();
    assert_eq!(h.sync().await.out_of_scope, 0, "leaving is recorded once");
    assert_eq!(h.counts(), before);

    // Back, task still live, with a new comment: resumes and delivers.
    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake.add_comment("AA-1", cm("c1", 10, "u1", "welcome back"));
    let st = h.sync().await;
    assert_eq!((st.resumed, st.tasks_created, st.comments_delivered), (1, 0, 1));
    assert_eq!(h.in_scope("AA-1"), vec![1]);
    assert_eq!(h.n("tasks"), 1);
    assert_eq!(h.task_id("AA-1").await, tid);
    assert!(h.task(&tid).body.contains("welcome back"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_that_stops_matching_a_rule_is_no_longer_followed() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let tid = h.task_id("AA-1").await;

    // Moves to a status no rule covers: still on the board, no longer auto-queued, link out of scope.
    h.fake.upsert(tk("AA-1", "In Progress", "s2"));
    let st = h.sync().await;
    assert_eq!(st.out_of_scope, 1);
    let v = h.ticket("AA-1").await.unwrap();
    assert!(!v.auto_queue);
    assert_eq!(h.in_scope("AA-1"), vec![0]);

    // Comments are not followed (not even fetched) while out of scope.
    h.fake.upsert(tk("AA-1", "In Progress", "s3"));
    h.fake
        .add_comment("AA-1", cm("c1", 10, "u1", "unseen while away"));
    let calls = h.fake.calls("comments:");
    let st = h.sync().await;
    assert_eq!((st.comments_delivered, st.tasks_created), (0, 0));
    assert_eq!(h.fake.calls("comments:"), calls);
    assert!(!h.task(&tid).body.contains("unseen while away"));

    // Back in a covered status: resumes, and the comment it missed is delivered.
    h.fake.upsert(tk("AA-1", "To Do", "s4"));
    let st = h.sync().await;
    assert_eq!((st.resumed, st.comments_delivered, st.tasks_created), (1, 1, 0));
    assert!(h.task(&tid).body.contains("unseen while away"));
    assert_eq!(h.in_scope("AA-1"), vec![1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_returning_after_its_task_finished_gets_a_fresh_task() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let old = h.task_id("AA-1").await;

    h.fake.remove("AA-1");
    h.sync().await;
    h.finish("worker");

    h.fake.upsert(tk("AA-1", "To Do", "s2"));
    h.fake
        .add_comment("AA-1", cm("c1", 10, "u1", "reassigned to me again"));
    let st = h.sync().await;
    assert_eq!((st.tasks_created, st.resumed), (1, 0));
    assert_eq!(h.n("tasks"), 2);
    let new = h.task_id("AA-1").await;
    assert_ne!(new, old);
    let t = h.task(&new);
    assert_eq!(t.status, TaskStatus::Open);
    assert!(t.body.contains("reassigned to me again"));
    assert_eq!(h.task(&old).status, TaskStatus::Done);
    assert_eq!(
        h.in_scope("AA-1"),
        vec![0, 1],
        "old link stays out of scope, new link is live"
    );
}

// ---------------------------------------------------------------------------------------------------------
// Rules, the board, done tickets
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn rules_are_scoped_by_project_and_uncovered_tickets_stay_on_the_board() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![
        tk("AA-1", "To Do", "s1"),
        tk("BB-1", "To Do", "s1"),
        tk("AA-2", "In Progress", "s1"),
    ]);
    let st = h.sync().await;
    assert_eq!((st.tickets, st.tasks_created), (3, 1));
    assert_eq!(h.n("tasks"), 1);
    let view = within(h.src.view(h.id)).await.unwrap();
    assert_eq!(view.tickets.len(), 3);
    for (key, queued) in [("AA-1", true), ("BB-1", false), ("AA-2", false)] {
        let v = view.tickets.iter().find(|t| t.key == key).unwrap();
        assert_eq!(v.task_id.is_some(), queued, "{key}");
        assert_eq!(v.auto_queue, queued, "{key}");
    }

    // Widening the rule to every project queues BB-1 (and still not the In Progress ticket).
    let mut f = default_filter();
    f.auto_queue = vec![AutoQueueRule {
        projects: vec!["*".into()],
        statuses: vec!["To Do".into()],
    }];
    let v = within(h.src.save_filter(h.id, f)).await.unwrap();
    assert_eq!(h.n("tasks"), 2);
    assert!(
        v.tickets
            .iter()
            .find(|t| t.key == "BB-1")
            .unwrap()
            .task_id
            .is_some()
    );
    assert!(
        v.tickets
            .iter()
            .find(|t| t.key == "AA-2")
            .unwrap()
            .task_id
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn done_tickets_never_auto_queue_but_appear_in_the_snapshot() {
    let h = h(SourceId::Jira);
    // A rule that (mistakenly) lists the done status must still not queue it.
    let mut f = default_filter();
    f.auto_queue = vec![AutoQueueRule {
        projects: vec!["*".into()],
        statuses: vec!["To Do".into(), "Done".into()],
    }];
    within(h.src.save_filter(h.id, f)).await.unwrap(); // pass runs against an empty tracker
    h.fake
        .set(vec![tk("AA-1", "Done", "s1"), tk("AA-2", "To Do", "s1")]);
    h.fake.add_comment("AA-1", cm("c1", 10, "u1", "closing remark"));
    let st = h.sync().await;
    assert_eq!((st.tickets, st.tasks_created), (2, 1));
    let done = h.ticket("AA-1").await.unwrap();
    assert_eq!(done.status_category, StatusCategory::Done);
    assert!(done.resolved_at.is_some());
    assert!(done.task_id.is_none() && !done.auto_queue);
    assert!(h.ticket("AA-2").await.unwrap().task_id.is_some());
    assert_eq!(
        h.fake.calls("comments:AA-1"),
        0,
        "comments of a never-queued ticket are not fetched"
    );
}

// ---------------------------------------------------------------------------------------------------------
// Manual queueing
// ---------------------------------------------------------------------------------------------------------

fn queue_req(k: &str) -> QueueTicket {
    QueueTicket { key: k.into() }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_queueing_rules() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![
        tk("BB-7", "To Do", "s1"), // on the board, not covered by a rule
        tk("AA-9", "Done", "s1"),
    ]);
    h.fake.add_comment("BB-7", cm("c1", 10, "u1", "context"));
    h.sync().await;
    assert_eq!(h.n("tasks"), 0);

    // Malformed keys, unknown keys and done tickets are rejected without writing anything.
    for bad in ["", "bb-7", "BB7", "BB-7/../x", "BB-"] {
        let e = within(h.src.queue(h.id, &queue_req(bad))).await.unwrap_err();
        assert!(matches!(e, CorvusError::Validation(_)), "{bad:?}: {e:?}");
    }
    let e = within(h.src.queue(h.id, &queue_req("ZZ-1"))).await.unwrap_err();
    assert!(matches!(e, CorvusError::NotFound(_)), "{e:?}");
    let e = within(h.src.queue(h.id, &queue_req("AA-9"))).await.unwrap_err();
    assert!(matches!(e, CorvusError::Conflict(_)), "{e:?}");
    assert_eq!(h.n("tasks"), 0);

    // Queue it by hand.
    let task = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap();
    assert_eq!(task.title, "[BB-7] Summary of BB-7");
    assert_eq!(task.status, TaskStatus::Open);
    assert_eq!(task.creator, "linear");
    assert!(task.body.contains(&untrusted_notice(SourceId::Linear)) && task.body.contains("context"));
    let v = h.ticket("BB-7").await.unwrap();
    assert!(v.manual && !v.auto_queue);
    assert_eq!(v.task_id.as_deref(), Some(task.id.as_str()));

    // A live task blocks a second queueing.
    let e = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap_err();
    assert!(matches!(e, CorvusError::Conflict(_)), "{e:?}");
    assert_eq!(h.n("tasks"), 1);

    // Manual links keep syncing comments while live, even though no rule covers the ticket.
    h.fake.upsert(tk("BB-7", "To Do", "s2"));
    h.fake
        .add_comment("BB-7", cm("c2", 20, "u1", "manual follow-through"));
    let st = h.sync().await;
    assert_eq!(st.comments_delivered, 1);
    assert!(h.task(&task.id).body.contains("manual follow-through"));
    assert_eq!(st.out_of_scope, 0);

    // ... and survive the ticket leaving the result set.
    h.fake.remove("BB-7");
    assert_eq!(h.sync().await.out_of_scope, 0);
    assert_eq!(h.in_scope("BB-7"), vec![1]);
    h.fake.upsert(tk("BB-7", "In Progress", "s3"));
    h.fake.add_comment("BB-7", cm("c3", 30, "u1", "after returning"));
    let st = h.sync().await;
    assert_eq!((st.comments_delivered, st.resumed), (1, 0));
    assert!(h.task(&task.id).body.contains("after returning"));
    assert!(h.ticket("BB-7").await.unwrap().manual);
}

/// BUG (engine): after the manual task finished, queueing the *unchanged* ticket again must create a new task.
/// The task idempotency key is `<source>:<KEY>:<updated_stamp>`; with an unchanged stamp it matches the finished
/// task, so `queue` returns that old finished task and creates no new task or link.
#[tokio::test(flavor = "multi_thread")]
async fn manual_queueing_is_allowed_again_after_the_task_finished() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("BB-7", "To Do", "s1")]);
    h.sync().await;
    let first = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap();
    assert_eq!(h.finish("worker"), first.id);

    // Same updated_stamp: nothing changed in the tracker since.
    let second = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap();
    assert_ne!(second.id, first.id, "a fresh task, not the finished one");
    assert_eq!(second.status, TaskStatus::Open);
    assert_eq!(h.n("tasks"), 2);
    assert_eq!(h.task_id("BB-7").await, second.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_queueing_after_finish_with_a_changed_stamp_creates_a_new_task() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("BB-7", "To Do", "s1")]);
    h.sync().await;
    let first = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap();
    h.cancel(&first.id);
    h.fake.upsert(tk("BB-7", "To Do", "s2"));
    h.sync().await;
    let second = within(h.src.queue(h.id, &queue_req("BB-7"))).await.unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(h.n("tasks"), 2);
    let v = h.ticket("BB-7").await.unwrap();
    assert!(v.manual);
    assert_eq!(v.task_id.as_deref(), Some(second.id.as_str()));
}

// ---------------------------------------------------------------------------------------------------------
// Partial views
// ---------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_search_writes_nothing_and_a_good_pass_clears_the_error() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.sync().await;
    let snapshot = within(h.src.view(h.id)).await.unwrap();
    let before = (h.counts(), h.work_events());

    h.fake.set(vec![
        tk("AA-1", "To Do", "s2"),
        tk("AA-2", "To Do", "s1"),
        tk("AA-3", "In Progress", "s1"),
    ]);
    h.fake.add_comment("AA-1", cm("c1", 10, "u1", "new remark"));
    h.fake.set_search_failing(true);
    let r = within(h.src.sync(h.id)).await.unwrap();
    assert!(!r.ok && r.stats.is_none() && r.error.as_deref().unwrap().contains("injected"));
    assert_eq!((h.counts(), h.work_events()).1, before.1, "no work events");
    assert_eq!((h.counts().0, h.counts().1, h.counts().3, h.counts().4), {
        let b = before.0;
        (b.0, b.1, b.3, b.4)
    });
    let after = within(h.src.view(h.id)).await.unwrap();
    assert_eq!(after.tickets, snapshot.tickets, "snapshot unchanged");
    assert_eq!(after.last_sync_at, snapshot.last_sync_at);
    assert!(after.last_error.as_deref().unwrap().contains("injected"));

    h.fake.set_search_failing(false);
    let st = h.sync().await;
    assert_eq!((st.tasks_created, st.comments_delivered), (1, 1));
    let good = within(h.src.view(h.id)).await.unwrap();
    assert!(good.last_error.is_none());
    assert_eq!(good.tickets.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_fetch_failing_partway_writes_nothing_at_all() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.fake.add_comment("AA-1", cm("c0", 1, "u1", "zero"));
    h.sync().await;
    let tid = h.task_id("AA-1").await;
    let body = h.task(&tid).body;
    let before = h.counts();
    let work = h.work_events();
    let snapshot = within(h.src.view(h.id)).await.unwrap().tickets;

    // Pass: search (call 1) ok, AA-1's comments (call 2) ok — and its comment would be delivered — but the
    // comments for the new ticket AA-2 (call 3) fail. Nothing from AA-1 may be written either.
    h.fake
        .set(vec![tk("AA-1", "To Do", "s2"), tk("AA-2", "To Do", "s1")]);
    h.fake
        .add_comment("AA-1", cm("c1", 10, "u1", "would be delivered"));
    h.fake.fail_in(3);
    let r = within(h.src.sync(h.id)).await.unwrap();
    assert!(!r.ok, "{r:?}");
    assert_eq!(
        h.fake.calls("comments:"),
        2 + 1,
        "AA-1 fetched earlier, then AA-1 and the failing AA-2"
    );
    assert_eq!(h.task(&tid).body, body);
    let after = h.counts();
    assert_eq!(
        (after.0, after.1, after.3, after.4),
        (before.0, before.1, before.3, before.4)
    );
    assert_eq!(h.work_events(), work);
    let v = within(h.src.view(h.id)).await.unwrap();
    assert_eq!(v.tickets, snapshot);
    assert!(v.last_error.is_some());

    let st = h.sync().await;
    assert_eq!((st.tasks_created, st.comments_delivered), (1, 1));
    assert!(h.task(&tid).body.contains("would be delivered"));
    assert!(within(h.src.view(h.id)).await.unwrap().last_error.is_none());
}

// ---------------------------------------------------------------------------------------------------------
// Single flight, and a hung tracker never holds the write lock
// ---------------------------------------------------------------------------------------------------------

fn alternates(log: &[String]) -> bool {
    // Every `search:start` is followed by its `search:end` before the next start.
    let mut open = false;
    for l in log {
        match l.as_str() {
            "search:start" => {
                if open {
                    return false;
                }
                open = true;
            }
            "search:end" => open = false,
            _ => {}
        }
    }
    true
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manual_sync_waits_for_a_running_pass_then_runs() {
    let h = h(SourceId::Jira);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.fake.hold(Hold::Search);

    let a = tokio::spawn({
        let (s, id) = (h.src.clone(), h.id);
        async move { s.sync(id).await }
    });
    h.fake.wait_entered().await;
    let b = tokio::spawn({
        let (s, id) = (h.src.clone(), h.id);
        async move { s.sync(id).await }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        h.fake.log(),
        vec!["search:start"],
        "the second pass has not started while the first is in flight"
    );
    assert!(!b.is_finished());

    h.fake.release();
    let (ra, rb) = within(async { (a.await.unwrap().unwrap(), b.await.unwrap().unwrap()) }).await;
    assert!(ra.ok && rb.ok);
    assert_eq!(
        ra.stats.unwrap().tasks_created + rb.stats.unwrap().tasks_created,
        1
    );
    let log = h.fake.log();
    assert_eq!(log.iter().filter(|l| *l == "search:start").count(), 2, "{log:?}");
    assert!(alternates(&log), "{log:?}");
    assert_eq!(h.n("tasks"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_processes_cannot_interleave_their_passes() {
    let t = TestDb::new();
    let state = t.dir.path().join("state");
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let (f1, f2) = (
        Fake::with_log(SourceId::Linear, log.clone()),
        Fake::with_log(SourceId::Linear, log.clone()),
    );
    for f in [&f1, &f2] {
        f.set(vec![tk("AA-1", "To Do", "s1")]);
    }
    // Two `Sources` on two database handles, one state dir: what two processes look like to the file lock.
    let s1 = Arc::new(Sources::new(
        t.services(),
        state.clone(),
        vec![f1.clone() as Arc<dyn WorkSource>],
    ));
    let s2 = Arc::new(Sources::new(
        t.services(),
        state,
        vec![f2.clone() as Arc<dyn WorkSource>],
    ));
    f1.hold(Hold::Search);

    let a = tokio::spawn({
        let s = s1.clone();
        async move { s.sync(SourceId::Linear).await }
    });
    f1.wait_entered().await;
    let b = tokio::spawn({
        let s = s2.clone();
        async move { s.sync(SourceId::Linear).await }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        *log.lock().unwrap(),
        vec!["search:start"],
        "the second instance is blocked on the file lock"
    );
    assert!(!b.is_finished());

    f1.release();
    let (ra, rb) = within(async { (a.await.unwrap().unwrap(), b.await.unwrap().unwrap()) }).await;
    assert!(ra.ok && rb.ok);
    let l = log.lock().unwrap().clone();
    assert_eq!(l.iter().filter(|x| *x == "search:start").count(), 2, "{l:?}");
    assert!(alternates(&l), "{l:?}");
    assert_eq!(
        count(&t.db, "tasks"),
        1,
        "the second pass found the first one's work"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hung_tracker_never_holds_the_database_write_lock() {
    for hold in [Hold::Search, Hold::Comments] {
        let h = h(SourceId::Jira);
        h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
        h.fake.hold(hold);
        let pass = tokio::spawn({
            let (s, id) = (h.src.clone(), h.id);
            async move { s.sync(id).await }
        });
        h.fake.wait_entered().await;

        // Another handle writes while the fetch is held open, promptly.
        let other = h.t.services();
        let started = std::time::Instant::now();
        let created = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                other.huginn().create_task(&CreateTask {
                    creator: "op".into(),
                    title: "written during the fetch".into(),
                    ..CreateTask::default()
                })
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("{hold:?}: the write blocked while the tracker was hung"))
        .unwrap()
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(created.title, "written during the fetch");
        assert!(!pass.is_finished(), "the pass is still waiting on the tracker");

        h.fake.release();
        let r = within(pass).await.unwrap().unwrap();
        assert!(r.ok);
        assert_eq!(h.n("tasks"), 2);
    }
}

// ---------------------------------------------------------------------------------------------------------
// Filter saving, unconfigured sources
// ---------------------------------------------------------------------------------------------------------

fn valid_filter() -> SourceFilter {
    SourceFilter {
        statuses: vec!["To Do".into()],
        done_days: 3,
        projects: vec!["AA".into()],
        auto_queue: vec![AutoQueueRule {
            projects: vec!["AA".into()],
            statuses: vec!["To Do".into()],
        }],
    }
}

fn stored_filter(h: &H) -> Option<String> {
    h.svc
        .db()
        .read(|tx| {
            use corvus_core::db::Reader;
            match Reader::sql(tx).query_row("SELECT filter FROM source_settings", [], |r| {
                r.get::<_, String>(0)
            }) {
                Ok(s) => Ok(Some(s)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(CorvusError::Internal(e.to_string())),
            }
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn save_filter_validates_stores_only_filter_fields_and_runs_a_pass() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);

    let mut bad = valid_filter();
    bad.statuses.clear();
    bad.done_days = 0;
    let e = within(h.src.save_filter(h.id, bad)).await.unwrap_err();
    assert!(matches!(e, CorvusError::Validation(_)), "{e:?}");
    let mut bad2 = valid_filter();
    bad2.projects = vec!["lower".into()];
    assert!(matches!(
        within(h.src.save_filter(h.id, bad2)).await.unwrap_err(),
        CorvusError::Validation(_)
    ));
    assert_eq!(h.n("source_settings"), 0, "nothing saved");
    assert!(h.fake.log().is_empty(), "no pass ran");
    assert_eq!(h.n("tasks"), 0);

    let v = within(h.src.save_filter(h.id, valid_filter())).await.unwrap();
    assert_eq!(v.filter, Some(valid_filter()));
    assert_eq!(h.n("source_settings"), 1);
    assert!(h.fake.calls("search") >= 1, "a pass ran");
    assert!(v.last_sync_at.is_some() && v.last_error.is_none());
    assert_eq!(h.n("tasks"), 1, "the saved rule queued the ticket");

    let json: serde_json::Value = serde_json::from_str(&stored_filter(&h).unwrap()).unwrap();
    let keys: HashSet<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(
        keys.is_subset(
            &["statuses", "done_days", "projects", "auto_queue"]
                .into_iter()
                .collect()
        ),
        "{keys:?}"
    );
    for rule in json["auto_queue"].as_array().unwrap() {
        let k: HashSet<&str> = rule.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(
            k.is_subset(&["projects", "statuses"].into_iter().collect()),
            "{k:?}"
        );
    }
    // The view reads it back, and a later pass uses it (not the default).
    assert_eq!(
        within(h.src.view(h.id)).await.unwrap().filter,
        Some(valid_filter())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_filter_stands_even_when_the_pass_it_triggers_fails() {
    let h = h(SourceId::Jira);
    h.fake.set_search_failing(true);
    let v = within(h.src.save_filter(h.id, valid_filter())).await.unwrap();
    assert_eq!(v.filter, Some(valid_filter()));
    assert!(v.last_error.is_some());
    assert_eq!(h.n("source_settings"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_that_is_not_configured_is_disabled_and_conflicts_on_mutation() {
    let t = TestDb::new();
    let fake = Fake::new(SourceId::Jira);
    let src = Sources::new(
        t.services(),
        t.dir.path().join("state"),
        vec![fake.clone() as Arc<dyn WorkSource>],
    );
    let list = src.list();
    assert_eq!(list.len(), 2);
    assert!(list.iter().find(|s| s.source == SourceId::Jira).unwrap().enabled);
    let lin = list.iter().find(|s| s.source == SourceId::Linear).unwrap();
    assert!(!lin.enabled && lin.site.is_none());

    let v = within(src.view(SourceId::Linear)).await.unwrap();
    assert!(!v.enabled && v.tickets.is_empty() && v.filter.is_none() && v.last_error.is_none());
    for e in [
        within(src.sync(SourceId::Linear)).await.unwrap_err(),
        within(src.save_filter(SourceId::Linear, valid_filter()))
            .await
            .unwrap_err(),
        within(src.queue(SourceId::Linear, &queue_req("AA-1")))
            .await
            .unwrap_err(),
    ] {
        assert!(
            matches!(&e, CorvusError::Conflict(m) if m.contains("not configured")),
            "{e:?}"
        );
    }
    assert_eq!(count(&t.db, "source_settings"), 0);
    assert!(fake.log().is_empty(), "the configured source was not touched");
}

// ---------------------------------------------------------------------------------------------------------
// The poll loop
// ---------------------------------------------------------------------------------------------------------

fn fast() -> LoopTimings {
    LoopTimings {
        first_pass: Duration::from_millis(30),
        interval: Duration::from_millis(60),
        leader_retry: Duration::from_millis(50),
        steps: STEPS,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_poll_loop_repeats_and_survives_a_failing_source() {
    let h = h(SourceId::Linear);
    h.fake.set(vec![tk("AA-1", "To Do", "s1")]);
    h.fake.set_search_failing(true);
    let passes = Arc::new(AtomicUsize::new(0));
    let (stop, rx) = watch::channel(false);
    let looped = tokio::spawn(run_background_with_sources(
        h.svc.clone(),
        h.t.dir.path().join("state"),
        "test",
        fast(),
        rx,
        {
            let p = passes.clone();
            move |_| {
                p.fetch_add(1, Ordering::SeqCst);
            }
        },
        Some(h.src.clone()),
    ));

    // The failing source is retried again and again; housekeeping keeps running next to it.
    wait_until("repeated failing polls", || {
        h.fake.search_calls.load(Ordering::SeqCst) >= 3
    })
    .await;
    wait_until("housekeeping passes", || passes.load(Ordering::SeqCst) >= 3).await;
    assert_eq!(h.n("tasks"), 0);
    assert!(within(h.src.view(h.id)).await.unwrap().last_error.is_some());
    assert!(!looped.is_finished(), "the loop did not die");

    // The tracker recovers: the next poll does the work without restarting anything.
    h.fake.set_search_failing(false);
    wait_until("recovery", || h.n("tasks") == 1).await;
    let calls = h.fake.search_calls.load(Ordering::SeqCst);
    wait_until("polling continues", || {
        h.fake.search_calls.load(Ordering::SeqCst) > calls + 1
    })
    .await;
    assert!(within(h.src.view(h.id)).await.unwrap().last_error.is_none());
    assert_eq!(h.n("tasks"), 1, "repeated polls stay idempotent");

    stop.send(true).unwrap();
    within(looped).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_background_leader_polls() {
    let t = TestDb::new();
    let state = t.dir.path().join("state");
    let mk = |_: usize| {
        let fake = Fake::new(SourceId::Jira);
        fake.set(vec![tk("AA-1", "To Do", "s1")]);
        let svc = t.services();
        let src = Arc::new(Sources::new(
            svc.clone(),
            state.clone(),
            vec![fake.clone() as Arc<dyn WorkSource>],
        ));
        (fake, svc, src)
    };
    let (f1, svc1, src1) = mk(1);
    let (f2, svc2, src2) = mk(2);
    let (stop1, rx1) = watch::channel(false);
    let (stop2, rx2) = watch::channel(false);
    let spawn = |svc: Services, rx, src| {
        tokio::spawn(run_background_with_sources(
            svc,
            state.clone(),
            "test",
            fast(),
            rx,
            |_| {},
            Some(src),
        ))
    };
    let l1 = spawn(svc1, rx1, src1);
    // Make the first loop the leader before the second even starts.
    wait_until("first loop polls", || f1.search_calls.load(Ordering::SeqCst) >= 2).await;
    let l2 = spawn(svc2, rx2, src2);

    let at = f1.search_calls.load(Ordering::SeqCst);
    wait_until("leader keeps polling", || {
        f1.search_calls.load(Ordering::SeqCst) >= at + 3
    })
    .await;
    assert_eq!(
        f2.search_calls.load(Ordering::SeqCst),
        0,
        "a non-leader never polls"
    );

    // When the leader stops, the other loop takes over.
    stop1.send(true).unwrap();
    within(l1).await.unwrap();
    wait_until("hand-over", || f2.search_calls.load(Ordering::SeqCst) >= 1).await;
    stop2.send(true).unwrap();
    within(l2).await.unwrap();
}
