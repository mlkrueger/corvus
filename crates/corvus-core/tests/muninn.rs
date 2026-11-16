//! Plan 0003: threads, messages, receipts, digests, decisions, bootstrap, connect, stats, overview.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::api::{
    AckMessage, CancelTask, ClaimTask, CreateTask, CreateThread, ListDecisions, ListMessages, MessageView,
    PostMessage, ReadInbox, RecordDecision, RegisterSession, SendDirect, WriteDigest,
};
use corvus_core::testing::{TestDb, count, exec, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{CorvusError, MessageType};

struct M {
    t: TestDb,
    s: Services,
}

impl M {
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
    fn thread(&self, creator: &str, title: &str) -> String {
        self.s
            .muninn()
            .create_thread(&CreateThread {
                creator: creator.into(),
                title: title.into(),
                ..Default::default()
            })
            .unwrap()
            .id
    }
    fn post(&self, thread: &str, from: &str, to: Option<&str>, kind: MessageType, body: &str) -> MessageView {
        self.post_reply(thread, from, to, kind, body, None).unwrap()
    }
    fn post_reply(
        &self,
        thread: &str,
        from: &str,
        to: Option<&str>,
        kind: MessageType,
        body: &str,
        in_reply_to: Option<i64>,
    ) -> Result<MessageView, CorvusError> {
        self.s.muninn().post_message(&PostMessage {
            thread_id: thread.into(),
            from: from.into(),
            body: body.into(),
            kind,
            to: to.map(Into::into),
            in_reply_to,
            artifacts: vec![],
            idempotency_key: None,
            session_id: None,
            wake: false,
        })
    }
    fn inbox(&self, agent: &str) -> Vec<i64> {
        self.inbox_after(agent, None)
    }
    fn inbox_after(&self, agent: &str, after: Option<i64>) -> Vec<i64> {
        self.s
            .muninn()
            .read_inbox(&ReadInbox {
                agent: agent.into(),
                limit: None,
                after_id: after,
            })
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }
    fn ack(&self, id: i64, agent: &str) -> Result<bool, CorvusError> {
        self.s
            .muninn()
            .ack_message(
                id,
                &AckMessage {
                    agent: agent.into(),
                    session_id: None,
                },
            )
            .map(|r| r.newly_acked)
    }
}

// ---------- threads ----------

#[test]
fn keyed_thread_is_idempotent_and_direct_prefix_reserved() {
    let m = M::new();
    let c = |key: &str| CreateThread {
        creator: "a".into(),
        title: "t".into(),
        key: Some(key.into()),
        ..Default::default()
    };
    let x = m.s.muninn().create_thread(&c("release-1.2")).unwrap();
    let y = m.s.muninn().create_thread(&c("release-1.2")).unwrap();
    assert_eq!(x.id, y.id);
    assert_eq!(
        query_i64(
            &m.t.db,
            "SELECT count(*) FROM events WHERE kind = 'thread.created'"
        ),
        Some(1)
    );
    assert!(matches!(
        m.s.muninn().create_thread(&c("direct:a:b")),
        Err(CorvusError::Validation(_))
    ));
    assert!(matches!(
        m.s.muninn().create_thread(&CreateThread {
            creator: "a".into(),
            title: " ".into(),
            ..Default::default()
        }),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn thread_for_task_returns_the_tasks_thread() {
    let m = M::new();
    let task =
        m.s.huginn()
            .create_task(&CreateTask {
                creator: "op".into(),
                title: "x".into(),
                ..Default::default()
            })
            .unwrap();
    let th =
        m.s.muninn()
            .create_thread(&CreateThread {
                creator: "a".into(),
                title: "t".into(),
                task_id: Some(task.id.clone()),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(Some(th.id), task.thread_id);
    let ghost = CreateThread {
        creator: "a".into(),
        title: "t".into(),
        task_id: Some("01890000-0000-7000-8000-000000000000".into()),
        ..Default::default()
    };
    assert!(matches!(
        m.s.muninn().create_thread(&ghost),
        Err(CorvusError::NotFound(_))
    ));
}

// ---------- unread / ack ----------

#[test]
fn unread_semantics() {
    let m = M::new();
    for a in ["alice", "bob", "carol"] {
        m.session(a);
    }
    let th = m.thread("alice", "t");
    let to_bob = m.post(&th, "alice", Some("bob"), MessageType::Note, "for bob");
    let to_carol = m.post(&th, "alice", Some("carol"), MessageType::Note, "for carol");
    let all = m.post(&th, "alice", None, MessageType::Status, "everyone");
    let from_bob = m.post(&th, "bob", None, MessageType::Note, "bob says");
    assert_eq!(
        m.inbox("bob"),
        vec![to_bob.id, all.id],
        "addressed + broadcast, not own, not others'"
    );
    assert_eq!(m.inbox("carol"), vec![to_carol.id, all.id, from_bob.id]);
    assert_eq!(m.inbox("alice"), vec![from_bob.id]);
    assert!(m.ack(to_bob.id, "bob").unwrap());
    assert_eq!(m.inbox("bob"), vec![all.id], "acked excluded");
    assert_eq!(
        m.inbox_after("carol", Some(all.id)),
        vec![from_bob.id],
        "after_id pages"
    );
    // Reading never acks.
    assert_eq!(count(&m.t.db, "receipts"), 1);
}

#[test]
fn after_id_is_only_a_paging_hint() {
    let m = M::new();
    m.session("bob");
    let th = m.thread("alice", "t");
    let first = m.post(&th, "alice", Some("bob"), MessageType::Note, "1");
    let second = m.post(&th, "alice", Some("bob"), MessageType::Note, "2");
    // A reader that paged past `second` still gets `first` back on a plain read while it is unacked.
    assert!(m.inbox_after("bob", Some(second.id)).is_empty());
    assert_eq!(m.inbox("bob"), vec![first.id, second.id]);
}

#[test]
fn ack_rules() {
    let m = M::new();
    m.session("bob");
    m.session("carol");
    let th = m.thread("alice", "t");
    let msg = m.post(&th, "alice", Some("bob"), MessageType::Question, "q");
    assert!(m.ack(msg.id, "bob").unwrap());
    assert!(!m.ack(msg.id, "bob").unwrap(), "idempotent; not newly acked");
    assert_eq!(
        query_i64(&m.t.db, "SELECT count(*) FROM events WHERE kind = 'inbox.acked'"),
        Some(1)
    );
    assert!(
        matches!(m.ack(msg.id, "alice"), Err(CorvusError::Conflict(_))),
        "own message"
    );
    assert!(
        matches!(m.ack(msg.id, "carol"), Err(CorvusError::Conflict(_))),
        "not addressed"
    );
    assert!(matches!(m.ack(99_999, "bob"), Err(CorvusError::NotFound(_))));
    let b = m.post(&th, "alice", None, MessageType::Note, "all");
    assert!(
        m.ack(b.id, "carol").unwrap(),
        "broadcast is addressed to everyone"
    );
}

#[test]
fn unresolved_question_needs_an_answer_reply() {
    let m = M::new();
    m.session("bob");
    let th = m.thread("alice", "t");
    let q = m.post(&th, "alice", Some("bob"), MessageType::Question, "which db?");
    let unresolved = |m: &M| m.s.muninn().bootstrap("bob").unwrap().unresolved_questions.len();
    assert_eq!(unresolved(&m), 1);
    m.post_reply(
        &th,
        "bob",
        Some("alice"),
        MessageType::Note,
        "thinking",
        Some(q.id),
    )
    .unwrap();
    assert_eq!(unresolved(&m), 1, "a note reply does not resolve");
    m.post(&th, "bob", Some("alice"), MessageType::Answer, "not a reply");
    assert_eq!(
        unresolved(&m),
        1,
        "an answer that is not a reply does not resolve"
    );
    m.post_reply(
        &th,
        "bob",
        Some("alice"),
        MessageType::Answer,
        "sqlite",
        Some(q.id),
    )
    .unwrap();
    assert_eq!(unresolved(&m), 0);
}

#[test]
fn reply_across_threads_is_a_validation_error() {
    let m = M::new();
    let (t1, t2) = (m.thread("a", "one"), m.thread("a", "two"));
    let q = m.post(&t1, "a", None, MessageType::Question, "q");
    let e = m
        .post_reply(&t2, "a", None, MessageType::Answer, "x", Some(q.id))
        .unwrap_err();
    assert!(matches!(e, CorvusError::Validation(_)), "{e:?}");
    assert!(matches!(
        m.post_reply(&t1, "a", None, MessageType::Answer, "x", Some(424_242)),
        Err(CorvusError::NotFound(_))
    ));
    // The store rejects a forced insert too.
    let forced = exec(
        &m.t.db,
        &format!(
            "INSERT INTO messages (thread_id, sender, type, in_reply_to, body, created_at) VALUES ('{t2}', 'a', 'answer', {}, 'x', 1)",
            q.id
        ),
    );
    assert!(forced.is_err());
}

#[test]
fn post_validation_and_not_found() {
    let m = M::new();
    let th = m.thread("a", "t");
    let base = || PostMessage {
        thread_id: th.clone(),
        from: "a".into(),
        body: "hi".into(),
        kind: MessageType::Note,
        to: None,
        in_reply_to: None,
        artifacts: vec![],
        idempotency_key: None,
        session_id: None,
        wake: false,
    };
    for bad in [
        PostMessage {
            body: " ".into(),
            ..base()
        },
        PostMessage {
            body: "x".repeat(20_001),
            ..base()
        },
        PostMessage {
            from: "a b".into(),
            ..base()
        },
    ] {
        assert!(matches!(
            m.s.muninn().post_message(&bad),
            Err(CorvusError::Validation(_))
        ));
    }
    assert!(matches!(
        m.s.muninn().post_message(&PostMessage {
            to: Some("ghost".into()),
            ..base()
        }),
        Err(CorvusError::NotFound(_))
    ));
    assert!(matches!(
        m.s.muninn().post_message(&PostMessage {
            thread_id: "01890000-0000-7000-8000-000000000000".into(),
            ..base()
        }),
        Err(CorvusError::NotFound(_))
    ));
    let s = m.session("b");
    assert!(
        matches!(
            m.s.muninn().post_message(&PostMessage {
                session_id: Some(s),
                ..base()
            }),
            Err(CorvusError::Conflict(_))
        ),
        "session of another agent"
    );
    assert_eq!(count(&m.t.db, "messages"), 0);
}

#[test]
fn message_event_has_no_body_and_idempotent_replay() {
    let m = M::new();
    let th = m.thread("a", "t");
    let p = |body: &str| PostMessage {
        thread_id: th.clone(),
        from: "a".into(),
        body: body.into(),
        kind: MessageType::Note,
        to: None,
        in_reply_to: None,
        artifacts: vec![],
        idempotency_key: Some("k".into()),
        session_id: None,
        wake: false,
    };
    let x = m.s.muninn().post_message(&p("SECRET-BODY")).unwrap();
    let y = m.s.muninn().post_message(&p("retry wording")).unwrap();
    assert_eq!(x.id, y.id);
    assert_eq!(y.body, "SECRET-BODY");
    assert_eq!(count(&m.t.db, "messages"), 1);
    assert_eq!(
        query_i64(
            &m.t.db,
            "SELECT count(*) FROM events WHERE kind = 'message.posted'"
        ),
        Some(1)
    );
    assert_eq!(
        query_i64(
            &m.t.db,
            "SELECT count(*) FROM events WHERE summary LIKE '%SECRET%'"
        ),
        Some(0)
    );
}

#[test]
fn direct_threads_pair_agents() {
    let m = M::new();
    for a in ["a", "b", "c"] {
        m.session(a);
    }
    let d = |from: &str, to: &str| {
        m.s.muninn()
            .send_direct(&SendDirect {
                from: from.into(),
                to: to.into(),
                body: "hi".into(),
                kind: MessageType::Note,
                idempotency_key: None,
                session_id: None,
            })
            .unwrap()
    };
    let ab = d("a", "b");
    let ba = d("b", "a");
    let ac = d("a", "c");
    assert_eq!(ab.thread_id, ba.thread_id);
    assert_ne!(ab.thread_id, ac.thread_id);
    assert_eq!(ab.recipient.as_deref(), Some("b"));
    assert_eq!(ba.recipient.as_deref(), Some("a"));
    let th = m.s.muninn().get_thread(&ab.thread_id).unwrap();
    assert_eq!(th.key.as_deref(), Some("direct:a:b"));
    assert!(matches!(
        m.s.muninn().send_direct(&SendDirect {
            from: "a".into(),
            to: "a".into(),
            body: "x".into(),
            kind: MessageType::Note,
            idempotency_key: None,
            session_id: None
        }),
        Err(CorvusError::Validation(_))
    ));
    assert!(matches!(
        m.s.muninn().send_direct(&SendDirect {
            from: "a".into(),
            to: "ghost".into(),
            body: "x".into(),
            kind: MessageType::Note,
            idempotency_key: None,
            session_id: None
        }),
        Err(CorvusError::NotFound(_))
    ));
}

#[test]
fn concurrent_first_direct_messages_create_one_thread() {
    let m = M::new();
    m.session("a");
    m.session("b");
    let handles: Vec<_> = (0..16).map(|_| m.t.services()).collect();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let joins: Vec<_> = handles
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                let (from, to) = if i % 2 == 0 { ("a", "b") } else { ("b", "a") };
                s.muninn()
                    .send_direct(&SendDirect {
                        from: from.into(),
                        to: to.into(),
                        body: "hi".into(),
                        kind: MessageType::Note,
                        idempotency_key: None,
                        session_id: None,
                    })
                    .unwrap()
                    .thread_id
            })
        })
        .collect();
    let ids: std::collections::HashSet<String> = joins.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(ids.len(), 1);
    assert_eq!(
        query_i64(&m.t.db, "SELECT count(*) FROM threads WHERE key = 'direct:a:b'"),
        Some(1)
    );
}

#[test]
fn list_messages_newest_first_with_read_lists_and_never_acks() {
    let m = M::new();
    for a in ["alice", "bob", "carol"] {
        m.session(a);
    }
    let th = m.thread("alice", "Plan");
    let other = m.thread("bob", "Other");
    let a1 = m.post(&th, "alice", None, MessageType::Note, "broadcast");
    let a2 = m.post(&th, "alice", Some("bob"), MessageType::Question, "q");
    let b1 = m.post(&other, "bob", Some("carol"), MessageType::Note, "x");
    m.ack(a1.id, "bob").unwrap();
    m.ack(b1.id, "carol").unwrap();
    let receipts = count(&m.t.db, "receipts");
    let l = m.s.muninn().list_messages(&ListMessages::default()).unwrap();
    assert_eq!(
        l.iter().map(|i| i.message.id).collect::<Vec<_>>(),
        vec![b1.id, a2.id, a1.id]
    );
    let bc = l.iter().find(|i| i.message.id == a1.id).unwrap();
    assert_eq!(bc.read_by, vec!["bob"]);
    assert_eq!(
        bc.unread_by,
        vec!["carol"],
        "audience = agents with sessions minus sender"
    );
    assert_eq!(bc.thread_title, "Plan");
    let page =
        m.s.muninn()
            .list_messages(&ListMessages {
                before_id: Some(a2.id),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(page.iter().map(|i| i.message.id).collect::<Vec<_>>(), vec![a1.id]);
    let by_thread =
        m.s.muninn()
            .list_messages(&ListMessages {
                thread: Some(other.clone()),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(by_thread.len(), 1);
    let carol =
        m.s.muninn()
            .list_messages(&ListMessages {
                agent: Some("carol".into()),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(
        carol.iter().map(|i| i.message.id).collect::<Vec<_>>(),
        vec![b1.id]
    );
    let unread =
        m.s.muninn()
            .list_messages(&ListMessages {
                unread_only: true,
                ..Default::default()
            })
            .unwrap();
    assert_eq!(
        unread.iter().map(|i| i.message.id).collect::<Vec<_>>(),
        vec![a2.id, a1.id]
    );
    assert_eq!(count(&m.t.db, "receipts"), receipts, "viewing never acks");
}

// ---------- memory ----------

#[test]
fn digests_latest_per_scope_key() {
    let m = M::new();
    let task =
        m.s.huginn()
            .create_task(&CreateTask {
                creator: "op".into(),
                title: "x".into(),
                ..Default::default()
            })
            .unwrap();
    let w = |scope: &str, r: Option<&str>, summary: &str| {
        m.s.muninn().write_digest(&WriteDigest {
            author: "alice".into(),
            scope: scope.into(),
            reference: r.map(Into::into),
            summary: summary.into(),
            ..Default::default()
        })
    };
    w("global", None, "g1").unwrap();
    w("global", None, "g2").unwrap();
    w("task", Some(&task.id), "t1").unwrap();
    w("agent", Some("alice"), "me").unwrap();
    assert_eq!(
        m.s.muninn()
            .latest_digest("global", None)
            .unwrap()
            .unwrap()
            .summary,
        "g2"
    );
    assert_eq!(
        m.s.muninn()
            .latest_digest("task", Some(&task.id))
            .unwrap()
            .unwrap()
            .summary,
        "t1"
    );
    let all = m.s.muninn().list_latest_digests().unwrap();
    assert_eq!(
        all.iter().map(|d| d.summary.as_str()).collect::<Vec<_>>(),
        vec!["me", "g2", "t1"]
    );
    assert!(matches!(
        w("task", Some("01890000-0000-7000-8000-000000000000"), "x"),
        Err(CorvusError::NotFound(_))
    ));
    assert!(matches!(
        w("agent", Some("ghost"), "x"),
        Err(CorvusError::NotFound(_))
    ));
    assert!(matches!(
        w("global", Some("x"), "x"),
        Err(CorvusError::Validation(_))
    ));
    assert!(matches!(w("task", None, "x"), Err(CorvusError::Validation(_))));
    assert!(matches!(w("global", None, "  "), Err(CorvusError::Validation(_))));
    assert!(matches!(
        m.s.muninn()
            .latest_digest("thread", Some("01890000-0000-7000-8000-000000000000")),
        Err(CorvusError::NotFound(_))
    ));
    assert_eq!(m.s.muninn().latest_digest("agent", Some("op")).unwrap(), None);
}

#[test]
fn digest_and_decision_idempotency() {
    let m = M::new();
    let d = |s: &str| WriteDigest {
        author: "a".into(),
        scope: "global".into(),
        summary: s.into(),
        idempotency_key: Some("k".into()),
        ..Default::default()
    };
    let x = m.s.muninn().write_digest(&d("one")).unwrap();
    let y = m.s.muninn().write_digest(&d("two")).unwrap();
    assert_eq!((x.id, y.summary.as_str()), (y.id, "one"));
    let r = |t: &str| RecordDecision {
        decided_by: "a".into(),
        title: t.into(),
        decision: "d".into(),
        idempotency_key: Some("k".into()),
        ..Default::default()
    };
    let p = m.s.muninn().record_decision(&r("first")).unwrap();
    let q = m.s.muninn().record_decision(&r("second")).unwrap();
    assert_eq!((p.id, q.title.as_str()), (q.id.clone(), "first"));
    assert_eq!(count(&m.t.db, "digests"), 1);
    assert_eq!(count(&m.t.db, "decisions"), 1);
    assert_eq!(
        query_i64(
            &m.t.db,
            "SELECT count(*) FROM events WHERE kind IN ('digest.written', 'decision.recorded')"
        ),
        Some(2)
    );
}

#[test]
fn decisions_superseded_hidden_by_default() {
    let m = M::new();
    let rec = |title: &str, sup: Option<&str>| {
        m.s.muninn()
            .record_decision(&RecordDecision {
                decided_by: "a".into(),
                title: title.into(),
                decision: "use x".into(),
                rationale: Some("because".into()),
                supersedes: sup.map(Into::into),
                ..Default::default()
            })
            .unwrap()
    };
    let old = rec("v1", None);
    m.t.clock.advance_ms(1);
    let new = rec("v2", Some(&old.id));
    let list = m.s.muninn().list_decisions(&ListDecisions::default()).unwrap();
    assert_eq!(
        list.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
        vec!["v2"]
    );
    let all =
        m.s.muninn()
            .list_decisions(&ListDecisions {
                include_superseded: true,
                ..Default::default()
            })
            .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        all.iter()
            .find(|d| d.id == old.id)
            .unwrap()
            .superseded_by
            .as_deref(),
        Some(new.id.as_str())
    );
    let bad = RecordDecision {
        decided_by: "a".into(),
        title: "t".into(),
        decision: " ".into(),
        ..Default::default()
    };
    assert!(matches!(
        m.s.muninn().record_decision(&bad),
        Err(CorvusError::Validation(_))
    ));
    let ghost = RecordDecision {
        decided_by: "a".into(),
        title: "t".into(),
        decision: "d".into(),
        supersedes: Some("01890000-0000-7000-8000-000000000000".into()),
        ..Default::default()
    };
    assert!(matches!(
        m.s.muninn().record_decision(&ghost),
        Err(CorvusError::NotFound(_))
    ));
}

#[test]
fn append_only_is_reasserted_for_messages_and_digests() {
    let m = M::new();
    let th = m.thread("a", "t");
    m.post(&th, "a", None, MessageType::Note, "x");
    m.s.muninn()
        .write_digest(&WriteDigest {
            author: "a".into(),
            scope: "global".into(),
            summary: "s".into(),
            ..Default::default()
        })
        .unwrap();
    for sql in [
        "UPDATE messages SET body = 'y'",
        "DELETE FROM messages",
        "UPDATE digests SET summary = 'y'",
        "DELETE FROM digests",
    ] {
        assert!(exec(&m.t.db, sql).is_err(), "{sql}");
    }
}

// ---------- bootstrap / connect / stats / overview ----------

#[test]
fn bootstrap_has_exactly_the_six_parts() {
    let m = M::new();
    let s = m.session("bob");
    let hg = m.s.huginn();
    let routed = hg
        .create_task(&CreateTask {
            creator: "op".into(),
            title: "for bob".into(),
            recipient: Some("bob".into()),
            ..Default::default()
        })
        .unwrap();
    hg.create_task(&CreateTask {
        creator: "op".into(),
        title: "anyone".into(),
        ..Default::default()
    })
    .unwrap();
    hg.create_task(&CreateTask {
        creator: "op".into(),
        title: "for carol".into(),
        recipient: Some("op".into()),
        ..Default::default()
    })
    .unwrap();
    let th = m.thread("alice", "t");
    m.post(&th, "alice", Some("bob"), MessageType::Question, "q?");
    m.s.muninn()
        .write_digest(&WriteDigest {
            author: "a".into(),
            scope: "global".into(),
            summary: "s".into(),
            ..Default::default()
        })
        .unwrap();
    m.s.muninn()
        .record_decision(&RecordDecision {
            decided_by: "a".into(),
            title: "t".into(),
            decision: "d".into(),
            ..Default::default()
        })
        .unwrap();
    let b = m.s.muninn().bootstrap("bob").unwrap();
    assert_eq!(
        b.routed_tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![routed.id.as_str()]
    );
    assert_eq!(
        b.claimable_tasks.len(),
        2,
        "routed-to-bob and unrouted; not routed to others"
    );
    assert_eq!(b.unread_messages.len(), 1);
    assert_eq!(b.unresolved_questions.len(), 1);
    assert_eq!(b.digests.len(), 1);
    assert_eq!(b.decisions.len(), 1);
    let json = serde_json::to_value(&b).unwrap();
    let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys.len(), 7, "agent + six parts: {keys:?}");
    // A task bob claims shows up as routed.
    hg.claim_task(&ClaimTask {
        session_id: s,
        pool: None,
    })
    .unwrap();
    assert!(!m.s.muninn().bootstrap("bob").unwrap().routed_tasks.is_empty());
}

#[test]
fn connect_is_session_plus_bootstrap_plus_guidance() {
    let m = M::new();
    let c =
        m.s.muninn()
            .connect(&RegisterSession {
                agent_name: Some("bob".into()),
                client_name: "c".into(),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(c.session.agent_name, "bob");
    assert_eq!(c.bootstrap.agent, "bob");
    assert_eq!(c.guidance, corvus_domain::GUIDANCE);
}

#[test]
fn stats_and_overview_match_a_seeded_scenario() {
    let m = M::new();
    let hg = m.s.huginn();
    let a = m.session("alice");
    let b = m.session("bob");
    let _quiet = m.session("quiet");
    let mk = |t: &str| {
        hg.create_task(&CreateTask {
            creator: "op".into(),
            title: t.into(),
            ..Default::default()
        })
        .unwrap()
    };
    let dep = mk("dep");
    hg.create_task(&CreateTask {
        creator: "op".into(),
        title: "child".into(),
        depends_on: vec![dep.id.clone()],
        ..Default::default()
    })
    .unwrap();
    mk("open-1");
    hg.cancel_task(
        &dep.id,
        &CancelTask {
            by: "op".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let c1 = hg
        .claim_task(&ClaimTask {
            session_id: a.clone(),
            pool: None,
        })
        .unwrap()
        .task
        .unwrap();
    let _c2 = mk("second");
    hg.claim_task(&ClaimTask {
        session_id: b.clone(),
        pool: None,
    })
    .unwrap();
    hg.complete_task(
        &c1.id,
        &corvus_core::api::CompleteTask {
            session_id: a.clone(),
            result_summary: None,
        },
    )
    .unwrap();
    let th = m.thread("op", "t");
    m.post(&th, "op", Some("bob"), MessageType::Note, "x");
    m.post(&th, "op", None, MessageType::Note, "all");

    m.t.clock.advance_secs(300); // alice/quiet idle; bob working (claim) → after heartbeat
    m.s.sessions().heartbeat(&b).unwrap();
    let st = m.s.overview().stats().unwrap();
    assert_eq!(
        (
            st.open_tasks,
            st.claimed_tasks,
            st.active_leases,
            st.expired_leases
        ),
        (1, 1, 1, 0)
    );
    assert_eq!(st.unread_by_agent["bob"], 2);
    assert_eq!(st.unread_by_agent["alice"], 1);
    assert_eq!(st.unread_total, 4);

    let o = m.s.overview().overview().unwrap();
    assert_eq!(o.operator_name, "operator");
    assert_eq!(o.presence_counts.get("working"), Some(&1));
    assert_eq!(o.presence_counts.get("idle"), Some(&2));
    assert_eq!(o.blocked_by_cancelled, 1);
    assert_eq!(o.tasks.open.len(), 1, "only the blocked child is open");
    assert!(o.tasks.open.iter().any(|t| t.blocked_by_cancelled));
    assert_eq!(o.tasks.claimed.len(), 1);
    assert_eq!(o.tasks.done.len(), 1);
    assert_eq!(o.tasks.cancelled.len(), 1);
    assert_eq!(
        o.sessions
            .iter()
            .find(|s| s.session.agent_name == "bob")
            .unwrap()
            .unread,
        2
    );
    assert!(!o.events.is_empty() && o.events.len() <= 100);
    assert_eq!(o.thresholds.lease_seconds, 900);
    assert_eq!(m.s.overview().health().unwrap().schema_version, 8);
}

/// Runs `f` on 16 threads (each with its own database handle), released together; returns the results.
fn concurrently<T: Send + 'static>(m: &M, f: impl Fn(&Services) -> T + Send + Sync + 'static) -> Vec<T> {
    let f = std::sync::Arc::new(f);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let joins: Vec<_> = (0..16)
        .map(|_| {
            let (s, b, f) = (m.t.services(), barrier.clone(), f.clone());
            std::thread::spawn(move || {
                b.wait();
                f(&s)
            })
        })
        .collect();
    joins.into_iter().map(|j| j.join().unwrap()).collect()
}

fn distinct<T: std::hash::Hash + Eq>(v: Vec<T>) -> usize {
    v.into_iter().collect::<std::collections::HashSet<_>>().len()
}

#[test]
fn concurrent_idempotent_creates_make_one_row_and_one_event() {
    let m = M::new();
    let th = m.thread("a", "base");
    let threads = concurrently(&m, |s| {
        s.muninn()
            .create_thread(&CreateThread {
                creator: "a".into(),
                title: "keyed".into(),
                key: Some("k".into()),
                ..Default::default()
            })
            .unwrap()
            .id
    });
    assert_eq!(distinct(threads), 1);
    let th2 = th.clone();
    let msgs = concurrently(&m, move |s| {
        s.muninn()
            .post_message(&PostMessage {
                thread_id: th2.clone(),
                from: "a".into(),
                body: "once".into(),
                kind: MessageType::Note,
                to: None,
                in_reply_to: None,
                artifacts: vec![],
                idempotency_key: Some("k".into()),
                session_id: None,
                wake: false,
            })
            .unwrap()
            .id
    });
    assert_eq!(distinct(msgs), 1);
    let digests = concurrently(&m, |s| {
        s.muninn()
            .write_digest(&WriteDigest {
                author: "a".into(),
                scope: "global".into(),
                summary: "s".into(),
                idempotency_key: Some("k".into()),
                ..Default::default()
            })
            .unwrap()
            .id
    });
    assert_eq!(distinct(digests), 1);
    let decisions = concurrently(&m, |s| {
        s.muninn()
            .record_decision(&RecordDecision {
                decided_by: "a".into(),
                title: "t".into(),
                decision: "d".into(),
                idempotency_key: Some("k".into()),
                ..Default::default()
            })
            .unwrap()
            .id
    });
    assert_eq!(distinct(decisions), 1);
    for (table, kind) in [
        ("messages", "message.posted"),
        ("digests", "digest.written"),
        ("decisions", "decision.recorded"),
    ] {
        assert_eq!(count(&m.t.db, table), 1, "{table}");
        assert_eq!(
            query_i64(
                &m.t.db,
                &format!("SELECT count(*) FROM events WHERE kind = '{kind}'")
            ),
            Some(1),
            "{kind}"
        );
    }
    assert_eq!(
        query_i64(&m.t.db, "SELECT count(*) FROM threads WHERE key = 'k'"),
        Some(1)
    );
    assert_eq!(
        query_i64(
            &m.t.db,
            "SELECT count(*) FROM events WHERE kind = 'thread.created'"
        ),
        Some(2)
    );
}

#[test]
fn hint_pruning_keeps_the_newest_rows() {
    let m = M::new();
    let th = m.thread("a", "t");
    for i in 0..25 {
        m.post(&th, "a", None, MessageType::Note, &format!("m{i}"));
    }
    let removed =
        m.t.db
            .write(|tx| corvus_core::store::hints::prune(tx, 10))
            .unwrap();
    assert_eq!(removed, 15);
    assert_eq!(count(&m.t.db, "hints"), 10);
    assert_eq!(query_i64(&m.t.db, "SELECT min(id) FROM hints"), Some(16));
    assert_eq!(
        m.t.db
            .write(|tx| corvus_core::store::hints::prune(tx, 10))
            .unwrap(),
        0
    );
}

// ---------- waking broadcasts (plan 0012, defaults #62) ----------

fn post_wake(m: &M, thread: &str, from: &str, to: Option<&str>) -> Result<MessageView, CorvusError> {
    m.s.muninn().post_message(&PostMessage {
        thread_id: thread.into(),
        from: from.into(),
        body: "everyone, stop and read this".into(),
        kind: MessageType::Note,
        to: to.map(str::to_owned),
        in_reply_to: None,
        artifacts: Vec::new(),
        idempotency_key: None,
        session_id: None,
        wake: true,
    })
}

#[test]
fn only_the_operators_broadcasts_can_wake() {
    let m = M::new();
    m.session("alice");
    m.session("bob");
    let thread = m.thread("operator", "Operator broadcasts");

    let woke = post_wake(&m, &thread, "operator", None).unwrap();
    assert!(woke.wake);
    assert_eq!(woke.recipient, None);
    assert_eq!(woke.reassigned_from, None);

    // Not from an agent, and not on a direct message even from the operator.
    for (from, to) in [("alice", None), ("operator", Some("bob")), ("alice", Some("bob"))] {
        let err = post_wake(&m, &thread, from, to).unwrap_err();
        assert!(
            matches!(err, CorvusError::Validation(_)),
            "{from} -> {to:?}: {err}"
        );
    }
    assert_eq!(count(&m.t.db, "messages"), 1, "the refused ones wrote nothing");

    // An ordinary message does not wake, and reads back that way.
    let plain = m.post(&thread, "operator", None, MessageType::Note, "fyi");
    assert!(!plain.wake);
    let inbox =
        m.s.muninn()
            .read_inbox(&ReadInbox {
                agent: "alice".into(),
                limit: None,
                after_id: None,
            })
            .unwrap();
    let flags: Vec<bool> = inbox.iter().map(|x| x.wake).collect();
    assert_eq!(flags, vec![true, false]);
}
