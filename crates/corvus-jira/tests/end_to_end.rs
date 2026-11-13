//! Plan 0009 step 6: the engine matrix end to end through the real Jira adapter and a fake Jira.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::sync::{Arc, Mutex};

use common::{Resp, serve, source};
use corvus_core::api::{ClaimTask, CompleteTask, ListTasks, QueueTicket, RegisterSession, TaskView};
use corvus_core::sources::Sources;
use corvus_core::testing::{TestDb, query_i64};
use corvus_domain::{SourceId, TaskStatus};
use serde_json::{Value, json};

#[derive(Default)]
struct Jira {
    issues: Vec<Value>,
    comments: std::collections::HashMap<String, Vec<Value>>,
}

fn adf(text: &str) -> Value {
    json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

fn issue(key: &str, status: &str, cat: &str, updated: &str) -> Value {
    let project = key.split('-').next().unwrap();
    json!({"key": key, "fields": {
        "summary": format!("Summary of {key}"),
        "status": {"name": status, "statusCategory": {"key": cat}},
        "priority": {"name": "High"},
        "updated": updated,
        "resolutiondate": null,
        "assignee": {"displayName": "Ada"},
        "project": {"key": project},
        "description": adf(&format!("Description of {key}")),
    }})
}

fn comment(id: u64, text: &str) -> Value {
    json!({"id": id.to_string(), "author": {"accountId": "acc-1", "displayName": "Bea"}, "body": adf(text),
           "created": "2026-01-01T00:00:00.000+0000"})
}

struct Rig {
    t: TestDb,
    jira: Arc<Mutex<Jira>>,
    sources: Sources,
}

async fn rig() -> Rig {
    let jira = Arc::new(Mutex::new(Jira::default()));
    let j = jira.clone();
    let (base, _log) = serve(move |req| {
        let state = j.lock().unwrap();
        if req.path.starts_with("/rest/api/3/search/jql") {
            Resp::json(json!({"issues": state.issues, "isLast": true}))
        } else if req.path.starts_with("/rest/api/3/status") {
            Resp::json(json!([{"name": "To Do", "statusCategory": {"key": "new"}}]))
        } else if let Some(rest) = req.path.strip_prefix("/rest/api/3/issue/") {
            let key = rest.split('/').next().unwrap();
            let cs = state.comments.get(key).cloned().unwrap_or_default();
            Resp::json(json!({"comments": cs, "total": cs.len(), "startAt": 0, "maxResults": 100}))
        } else {
            Resp::status(404)
        }
    })
    .await;
    let t = TestDb::new();
    let src = source(&base);
    let sources = Sources::new(t.services(), t.dir.path().join("state"), vec![Arc::new(src)]);
    Rig { t, jira, sources }
}

impl Rig {
    async fn sync(&self) -> corvus_core::api::SyncStats {
        let r = self.sources.sync(SourceId::Jira).await.unwrap();
        assert!(r.ok, "{:?}", r.error);
        r.stats.unwrap()
    }
    fn tasks(&self) -> Vec<TaskView> {
        self.t
            .services()
            .huginn()
            .list_tasks(&ListTasks::default())
            .unwrap()
    }
    fn task_for(&self, key: &str) -> TaskView {
        self.tasks()
            .into_iter()
            .find(|t| t.title.starts_with(&format!("[{key}]")))
            .unwrap()
    }
    fn set(&self, f: impl FnOnce(&mut Jira)) {
        f(&mut self.jira.lock().unwrap());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn assignment_idempotency_scoping_and_comment_routing() {
    let r = rig().await;
    // AA-1 matches the default rule (project AA, To Do); BB-1 is shown but no rule covers it.
    r.set(|j| {
        j.issues = vec![
            issue("AA-1", "To Do", "new", "u1"),
            issue("BB-1", "To Do", "new", "u1"),
        ];
        j.comments
            .insert("AA-1".into(), vec![comment(10, "first comment")]);
    });
    let s = r.sync().await;
    assert_eq!((s.tickets, s.tasks_created), (2, 1));
    let task = r.task_for("AA-1");
    assert_eq!(task.creator, "jira");
    assert_eq!(task.priority, 5, "High → 5");
    assert!(task.body.contains("UNTRUSTED text copied from Jira"));
    assert!(task.body.contains("Description of AA-1") && task.body.contains("first comment"));
    assert!(task.body.contains("/browse/AA-1"));
    assert_eq!(r.tasks().len(), 1, "BB-1 is on the board but not queued");

    // Idempotent re-poll.
    let events = query_i64(&r.t.db, "SELECT count(*) FROM events").unwrap();
    let s = r.sync().await;
    assert_eq!((s.tasks_created, s.comments_delivered), (0, 0));
    assert_eq!(query_i64(&r.t.db, "SELECT count(*) FROM events").unwrap(), events);

    // Open task: a new comment is appended.
    r.set(|j| {
        j.issues[0] = issue("AA-1", "To Do", "new", "u2");
        j.comments
            .get_mut("AA-1")
            .unwrap()
            .push(comment(11, "second comment"));
    });
    assert_eq!(r.sync().await.comments_delivered, 1);
    let body = r.task_for("AA-1").body;
    assert!(body.contains("second comment"));
    assert_eq!(
        body.matches("first comment").count(),
        1,
        "old comments are not repeated"
    );

    // Claimed task: the next comment becomes one message to the claimant.
    let s = r.t.services();
    let sid = s
        .sessions()
        .register(&RegisterSession {
            agent_name: Some("dev".into()),
            client_name: "t".into(),
            ..Default::default()
        })
        .unwrap()
        .id;
    s.huginn()
        .claim_task(&ClaimTask {
            session_id: sid.clone(),
            pool: None,
        })
        .unwrap();
    r.set(|j| {
        j.issues[0] = issue("AA-1", "In Progress", "indeterminate", "u3");
        j.comments
            .get_mut("AA-1")
            .unwrap()
            .push(comment(12, "third comment"));
    });
    assert_eq!(r.sync().await.comments_delivered, 1);
    let inbox = s
        .muninn()
        .read_inbox(&corvus_core::api::ReadInbox {
            agent: "dev".into(),
            limit: None,
            after_id: None,
        })
        .unwrap();
    assert_eq!(inbox.len(), 1);
    assert!(inbox[0].body.contains("third comment") && inbox[0].body.contains("UNTRUSTED"));

    // Done task: the next comment becomes a follow-up task.
    let id = r.task_for("AA-1").id;
    s.huginn()
        .complete_task(
            &id,
            &CompleteTask {
                session_id: sid,
                result_summary: None,
            },
        )
        .unwrap();
    r.set(|j| {
        j.issues[0] = issue("AA-1", "In Progress", "indeterminate", "u4");
        j.comments
            .get_mut("AA-1")
            .unwrap()
            .push(comment(13, "fourth comment"));
    });
    assert_eq!(r.sync().await.followups, 1);
    let follow = r
        .tasks()
        .into_iter()
        .find(|t| t.title.contains("Follow-up"))
        .unwrap();
    assert!(follow.body.contains("fourth comment"));
    assert_eq!(follow.status, TaskStatus::Open);

    // Re-assignment: AA-1 leaves the result set → out of scope.
    r.set(|j| j.issues.retain(|i| i["key"] != "AA-1"));
    assert_eq!(r.sync().await.out_of_scope, 1);
    let view = r.sources.view(SourceId::Jira).await.unwrap();
    assert!(view.tickets.iter().all(|t| t.key != "AA-1"));

    // Manual queueing of BB-1, and its comment sync.
    let manual = r
        .sources
        .queue(SourceId::Jira, &QueueTicket { key: "BB-1".into() })
        .await
        .unwrap();
    assert!(manual.title.starts_with("[BB-1]"));
    let err = r
        .sources
        .queue(SourceId::Jira, &QueueTicket { key: "BB-1".into() })
        .await
        .unwrap_err();
    assert!(
        matches!(err, corvus_domain::CorvusError::Conflict(_)),
        "live task already exists"
    );
    r.set(|j| {
        j.issues[0] = issue("BB-1", "To Do", "new", "u5");
        j.comments
            .insert("BB-1".into(), vec![comment(20, "manual comment")]);
    });
    assert_eq!(r.sync().await.comments_delivered, 1);
    assert!(r.task_for("BB-1").body.contains("manual comment"));
    let view = r.sources.view(SourceId::Jira).await.unwrap();
    let bb = view.tickets.iter().find(|t| t.key == "BB-1").unwrap();
    assert!(bb.manual && !bb.auto_queue);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_tracker_writes_nothing() {
    let jira_ok = Arc::new(Mutex::new(true));
    let ok = jira_ok.clone();
    let (base, _log) = serve(move |req| {
        if !*ok.lock().unwrap() {
            return Resp::status(503);
        }
        if req.path.starts_with("/rest/api/3/search/jql") {
            Resp::json(json!({"issues": [issue("AA-1", "To Do", "new", "u1")], "isLast": true}))
        } else {
            Resp::json(json!({"comments": [], "total": 0, "startAt": 0, "maxResults": 100}))
        }
    })
    .await;
    let t = TestDb::new();
    let sources = Sources::new(
        t.services(),
        t.dir.path().join("state"),
        vec![Arc::new(source(&base))],
    );
    *jira_ok.lock().unwrap() = false;
    let r = sources.sync(SourceId::Jira).await.unwrap();
    assert!(!r.ok && r.error.unwrap().contains("503"));
    assert_eq!(query_i64(&t.db, "SELECT count(*) FROM tasks"), Some(0));
    assert!(sources.view(SourceId::Jira).await.unwrap().last_error.is_some());
    *jira_ok.lock().unwrap() = true;
    assert!(sources.sync(SourceId::Jira).await.unwrap().ok);
    assert_eq!(query_i64(&t.db, "SELECT count(*) FROM tasks"), Some(1));
    assert!(sources.view(SourceId::Jira).await.unwrap().last_error.is_none());
}
