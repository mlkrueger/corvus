//! Plan 0012 step 9: reassigning stranded mail (D0025) and "reachable" (system/06), on real SQLite files.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::api::{
    AckMessage, ArtifactInput, ConversationInput, CreateThread, ListMessages, MessageView, PostMessage,
    ReadInbox, ReassignMail, RegisterSession, SendDirect,
};
use corvus_core::delivery::watch::WatchLock;
use corvus_core::testing::{TestDb, count, exec, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{AgentName, CorvusError, MessageType};

struct R {
    t: TestDb,
    s: Services,
}

impl R {
    fn new() -> Self {
        let t = TestDb::new();
        let s = t.services_with(Settings::default());
        Self { t, s }
    }
    fn session(&self, agent: &str) -> String {
        self.s
            .sessions()
            .register(&RegisterSession {
                agent_name: Some(agent.into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap()
            .id
    }
    /// An agent that has had a session and has none now.
    fn offline(&self, agent: &str) {
        let id = self.session(agent);
        self.s.sessions().end(&id).unwrap();
    }
    fn direct(&self, from: &str, to: &str, kind: MessageType, body: &str) -> MessageView {
        self.s
            .muninn()
            .send_direct(&SendDirect {
                from: from.into(),
                to: to.into(),
                body: body.into(),
                kind,
                idempotency_key: None,
                session_id: None,
            })
            .unwrap()
    }
    fn inbox(&self, agent: &str) -> Vec<MessageView> {
        self.s
            .muninn()
            .read_inbox(&ReadInbox {
                agent: agent.into(),
                limit: None,
                after_id: None,
            })
            .unwrap()
    }
    fn reassign(&self, from: &str, to: &str) -> Result<Vec<(i64, i64)>, CorvusError> {
        self.s
            .muninn()
            .reassign_mail(&ReassignMail {
                from: from.into(),
                to: to.into(),
            })
            .map(|r| r.moved.iter().map(|m| (m.original_id, m.message_id)).collect())
    }
}

#[test]
fn copies_unread_direct_mail_and_marks_the_originals() {
    let r = R::new();
    r.offline("old");
    r.session("new");
    r.session("carol");
    let q = r.direct("carol", "old", MessageType::Question, "where is the schema?");
    let n = r.direct("operator", "old", MessageType::Note, "second");
    let read = r.direct("carol", "old", MessageType::Note, "already read");
    r.s.muninn()
        .ack_message(
            read.id,
            &AckMessage {
                agent: "old".into(),
                session_id: None,
            },
        )
        .unwrap();
    let from_new = r.direct("new", "old", MessageType::Note, "from the new agent itself");
    // A broadcast is not moved: `new` already has its own copy.
    let thread =
        r.s.muninn()
            .create_thread(&CreateThread {
                creator: "carol".into(),
                title: "all".into(),
                ..Default::default()
            })
            .unwrap();
    let broadcast =
        r.s.muninn()
            .post_message(&PostMessage {
                thread_id: thread.id,
                from: "carol".into(),
                body: "to everyone".into(),
                kind: MessageType::Note,
                to: None,
                in_reply_to: None,
                artifacts: vec![],
                idempotency_key: None,
                session_id: None,
                wake: false,
            })
            .unwrap();
    let events_before = count(&r.t.db, "events");

    let moved = r.reassign("old", "new").unwrap();
    assert_eq!(
        moved.iter().map(|m| m.0).collect::<Vec<_>>(),
        [q.id, n.id],
        "unread direct mail only, oldest first"
    );

    // The new agent has ordinary unread copies: same thread, type, sender and body.
    let inbox = r.inbox("new");
    let copies: Vec<&MessageView> = inbox.iter().filter(|m| m.reassigned_from.is_some()).collect();
    assert_eq!(copies.len(), 2);
    assert_eq!(copies[0].id, moved[0].1);
    assert_eq!(copies[0].reassigned_from, Some(q.id));
    assert_eq!(copies[0].sender, "carol");
    assert_eq!(copies[0].recipient.as_deref(), Some("new"));
    assert_eq!(copies[0].kind, MessageType::Question);
    assert_eq!(copies[0].body, "where is the schema?");
    assert_eq!(copies[0].thread_id, q.thread_id);
    assert_eq!(copies[1].reassigned_from, Some(n.id));
    assert_eq!(copies[1].sender, "operator");
    assert!(inbox.iter().any(|m| m.id == broadcast.id));

    // The old agent's inbox keeps only what was not moved: the new agent's own message and the broadcast.
    let left: Vec<i64> = r.inbox("old").iter().map(|m| m.id).collect();
    assert_eq!(left, [from_new.id, broadcast.id]);

    // One receipt per original, of the distinct kind; one event per message, with no body.
    assert_eq!(
        query_i64(
            &r.t.db,
            "SELECT count(*) FROM receipts WHERE kind = 'reassigned' AND agent_name = 'old' AND reassigned_to = 'new'"
        ),
        Some(2)
    );
    assert_eq!(count(&r.t.db, "events") - events_before, 2);
    assert_eq!(
        query_i64(
            &r.t.db,
            "SELECT count(*) FROM events WHERE kind = 'message.reassigned' AND agent_name = 'operator'
               AND summary NOT LIKE '%schema%' AND summary NOT LIKE '%second%'"
        ),
        Some(2)
    );

    // Idempotent: nothing is unread any more.
    let messages = count(&r.t.db, "messages");
    assert_eq!(r.reassign("old", "new").unwrap(), []);
    assert_eq!(count(&r.t.db, "messages"), messages);
    assert_eq!(count(&r.t.db, "events") - events_before, 2);
}

#[test]
fn a_reassignment_is_not_an_acknowledgement_in_the_operator_view() {
    let r = R::new();
    r.offline("old");
    r.session("new");
    let original = r.direct("operator", "old", MessageType::Note, "hello");
    let moved = r.reassign("old", "new").unwrap();
    let list = r.s.muninn().list_messages(&ListMessages::default()).unwrap();
    let find = |id: i64| list.iter().find(|m| m.message.id == id).unwrap();

    let o = find(original.id);
    assert_eq!(o.read_by, Vec::<String>::new(), "never 'read by old'");
    assert_eq!(o.unread_by, Vec::<String>::new());
    assert_eq!(o.reassigned_to.as_deref(), Some("new"));

    let c = find(moved[0].1);
    assert_eq!(c.unread_by, ["new"]);
    assert_eq!(c.reassigned_to, None);
    assert_eq!(c.message.reassigned_from, Some(original.id));

    // Only the unread copy is still unread.
    let unread =
        r.s.muninn()
            .list_messages(&ListMessages {
                unread_only: true,
                ..Default::default()
            })
            .unwrap();
    assert_eq!(
        unread.iter().map(|m| m.message.id).collect::<Vec<_>>(),
        [moved[0].1]
    );
    let stats = r.s.overview().stats().unwrap();
    assert_eq!(stats.unread_by_agent.get("old"), Some(&0));
    assert_eq!(stats.unread_by_agent.get("new"), Some(&1));

    // The new agent acks its copy as usual; the old agent acking the original changes nothing.
    let ack = |id: i64, agent: &str| {
        r.s.muninn()
            .ack_message(
                id,
                &AckMessage {
                    agent: agent.into(),
                    session_id: None,
                },
            )
            .unwrap()
            .newly_acked
    };
    assert!(ack(moved[0].1, "new"));
    assert!(!ack(original.id, "old"));
    assert_eq!(
        query_i64(&r.t.db, "SELECT count(*) FROM receipts WHERE kind = 'ack'"),
        Some(1)
    );
}

#[test]
fn copies_keep_replies_and_artifacts() {
    let r = R::new();
    r.offline("old");
    r.session("new");
    let first = r.direct("operator", "old", MessageType::Question, "q");
    let reply =
        r.s.muninn()
            .post_message(&PostMessage {
                thread_id: first.thread_id.clone(),
                from: "operator".into(),
                body: "see the file".into(),
                kind: MessageType::Note,
                to: Some("old".into()),
                in_reply_to: Some(first.id),
                artifacts: vec![ArtifactInput {
                    kind: corvus_domain::ArtifactKind::File,
                    path: "src/lib.rs".into(),
                    commit: None,
                    description: Some("the file".into()),
                }],
                idempotency_key: None,
                session_id: None,
                wake: false,
            })
            .unwrap();
    let moved = r.reassign("old", "new").unwrap();
    let inbox = r.inbox("new");
    let copy = inbox
        .iter()
        .find(|m| m.reassigned_from == Some(reply.id))
        .unwrap();
    assert_eq!(copy.id, moved[1].1);
    assert_eq!(copy.in_reply_to, Some(first.id));
    assert_eq!(copy.artifacts.len(), 1);
    assert_eq!(copy.artifacts[0].path, "src/lib.rs");
    assert_ne!(copy.artifacts[0].id, reply.artifacts[0].id);
}

#[test]
fn validation_and_unknown_agents() {
    let r = R::new();
    r.offline("old");
    assert_eq!(r.reassign("old", "old").unwrap_err().code(), "validation");
    assert_eq!(r.reassign("old", "Not A Name").unwrap_err().code(), "validation");
    assert_eq!(r.reassign("old", "ghost").unwrap_err().code(), "not_found");
    assert_eq!(r.reassign("ghost", "old").unwrap_err().code(), "not_found");
    assert_eq!(count(&r.t.db, "receipts"), 0);
}

#[test]
fn a_failure_part_way_moves_nothing() {
    let r = R::new();
    r.offline("old");
    r.session("new");
    r.direct("operator", "old", MessageType::Note, "one");
    r.direct("operator", "old", MessageType::Note, "two");
    // Make the second event insert fail: the whole reassignment must roll back.
    exec(
        &r.t.db,
        "CREATE TRIGGER fail_second BEFORE INSERT ON events
           WHEN NEW.kind = 'message.reassigned'
            AND (SELECT count(*) FROM receipts WHERE kind = 'reassigned') = 2
         BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .unwrap();
    let messages = count(&r.t.db, "messages");
    assert!(r.reassign("old", "new").is_err());
    assert_eq!(count(&r.t.db, "messages"), messages);
    assert_eq!(count(&r.t.db, "receipts"), 0);
    assert_eq!(r.inbox("old").len(), 2);
}

#[test]
fn reachable_is_a_session_a_watcher_or_a_waiter() {
    let r = R::new();
    let state = tempfile::tempdir().unwrap();
    let s = r.s.clone().with_state_dir(state.path().to_path_buf());
    let status = |name: &str| s.overview().agent_status(name).unwrap();

    r.offline("old");
    r.session("live");
    r.direct("operator", "old", MessageType::Note, "one");
    assert!(status("live").reachable);
    let old = status("old");
    assert!(!old.reachable);
    assert_eq!(old.unread_direct, 1);
    assert_eq!(
        s.overview().agent_status("ghost").unwrap_err().code(),
        "not_found"
    );

    // A watcher holding the agent's lock makes it reachable, for as long as it holds it.
    let lock = WatchLock::acquire(state.path(), &AgentName::parse("old").unwrap()).unwrap();
    assert!(status("old").reachable);
    drop(lock);
    assert!(!status("old").reachable);

    // A waiter on a conversation linked to the agent makes it reachable.
    let conversation = "0a0a0a0a-1111-4222-8333-444444444444";
    let id =
        r.s.sessions()
            .register(&RegisterSession {
                agent_name: Some("linked".into()),
                client_name: "claude-code".into(),
                conversation: Some(ConversationInput {
                    tool: "claude-code".into(),
                    id: conversation.into(),
                    directory: Some("/work/project".into()),
                }),
                ..Default::default()
            })
            .unwrap()
            .id;
    r.s.sessions().end(&id).unwrap();
    assert!(!status("linked").reachable);
    let pid = 4242u32;
    let current = corvus_core::delivery::hook::current_path(state.path(), pid);
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    std::fs::write(&current, format!("{{\"conversation\":\"{conversation}\"}}")).unwrap();
    let lock_path = corvus_core::delivery::waiter::lock_path(state.path(), pid);
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    std::fs::write(&lock_path, "1").unwrap();
    assert!(
        !status("linked").reachable,
        "an unheld lock file is nobody waiting"
    );
    let waiter = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    waiter.try_lock().unwrap();
    let linked = status("linked");
    assert!(linked.reachable);
    assert_eq!(linked.conversations.len(), 1);
    assert_eq!(
        linked.conversations[0].directory.as_deref(),
        Some("/work/project")
    );
    assert!(!status("old").reachable, "someone else's waiter");

    // The overview carries the same list; an agent that only ever sent mail is not on it.
    let overview = s.overview().overview().unwrap();
    let names: Vec<&str> = overview.agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["linked", "live", "old"]);
    let by = |n: &str| overview.agents.iter().find(|a| a.name == n).unwrap();
    assert!(by("linked").reachable && by("live").reachable && !by("old").reachable);
    assert_eq!(by("old").unread_direct, 1);
    assert_eq!(s.overview().agents().unwrap(), overview.agents);
    drop(waiter);

    // With no state directory configured no lock is ever seen.
    assert!(!r.s.overview().agent_status("linked").unwrap().reachable);
}
