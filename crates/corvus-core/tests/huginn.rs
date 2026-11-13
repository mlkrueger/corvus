//! Plan 0002: tasks, claims, leases, dependencies, cancellation (functional; races live in races.rs).
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use corvus_core::api::{
    ArtifactInput, CancelTask, ClaimResult, ClaimTask, CompleteTask, CreateTask, ListTasks, RegisterSession,
    TaskView,
};
use corvus_core::testing::{TestDb, count, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{ArtifactKind, CorvusError, EndReason, TaskStatus};

struct H {
    t: TestDb,
    s: Services,
}

impl H {
    fn new() -> Self {
        let t = TestDb::new();
        let s = t.services_with(Settings {
            lease_ms: 900_000,
            ..Settings::default()
        });
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
    fn create(&self, title: &str) -> TaskView {
        self.create_with(CreateTask {
            title: title.into(),
            ..Default::default()
        })
    }
    fn create_with(&self, mut c: CreateTask) -> TaskView {
        if c.creator.is_empty() {
            c.creator = "op".into();
        }
        self.s.huginn().create_task(&c).unwrap()
    }
    fn claim(&self, session: &str) -> ClaimResult {
        self.claim_pool(session, None)
    }
    fn claim_pool(&self, session: &str, pool: Option<&str>) -> ClaimResult {
        self.s
            .huginn()
            .claim_task(&ClaimTask {
                session_id: session.into(),
                pool: pool.map(Into::into),
            })
            .unwrap()
    }
    fn claimed_title(&self, session: &str) -> Option<String> {
        self.claim(session).task.map(|t| t.title)
    }
    fn events(&self) -> i64 {
        count(&self.t.db, "events")
    }
    fn complete(&self, task: &str, session: &str) -> Result<TaskView, CorvusError> {
        self.s.huginn().complete_task(
            task,
            &CompleteTask {
                session_id: session.into(),
                result_summary: Some("shipped".into()),
            },
        )
    }
    fn cancel(&self, task: &str, cascade: bool) -> Result<corvus_core::api::CancelResult, CorvusError> {
        self.s.huginn().cancel_task(
            task,
            &CancelTask {
                by: "op".into(),
                reason: Some("scope cut".into()),
                cascade,
                session_id: None,
            },
        )
    }
}

// ---------- create ----------

#[test]
fn create_makes_task_thread_and_one_event() {
    let h = H::new();
    let before = h.events();
    let t = h.create_with(CreateTask {
        creator: "alice".into(),
        title: "  Fix login  ".into(),
        body: Some("details".into()),
        priority: Some(3),
        pool: Some("backend".into()),
        artifacts: vec![ArtifactInput {
            kind: ArtifactKind::File,
            path: "src/login.rs".into(),
            commit: Some("abcdef1".into()),
            description: Some("entry point".into()),
        }],
        ..Default::default()
    });
    assert_eq!(t.title, "Fix login");
    assert_eq!(
        (t.status, t.priority, t.pool.as_str()),
        (TaskStatus::Open, 3, "backend")
    );
    assert!(t.thread_id.is_some());
    assert_eq!(t.artifacts.len(), 1);
    assert_eq!(t.artifacts[0].commit.as_deref(), Some("abcdef1"));
    assert_eq!(count(&h.t.db, "threads"), 1);
    assert_eq!(h.events(), before + 1);
    let summary: String =
        h.t.db
            .read(|tx| {
                corvus_core::db::Reader::sql(tx)
                    .query_row("SELECT summary FROM events ORDER BY id DESC LIMIT 1", [], |r| {
                        r.get(0)
                    })
                    .map_err(corvus_core::db::sql_err)
            })
            .unwrap();
    assert!(
        !summary.contains("details"),
        "event summaries never carry bodies: {summary}"
    );
}

#[test]
fn create_defaults() {
    let h = H::new();
    let t = h.create("x");
    assert_eq!(
        (t.priority, t.pool.as_str(), t.body.as_str(), t.creator.as_str()),
        (0, "default", "", "op")
    );
    assert!(t.recipient_agent.is_none() && t.depends_on.is_empty() && !t.blocked);
}

#[test]
fn create_validation_and_not_found() {
    let h = H::new();
    let hg = h.s.huginn();
    let base = || CreateTask {
        creator: "op".into(),
        title: "t".into(),
        ..Default::default()
    };
    let validation = [
        CreateTask {
            title: "  ".into(),
            ..base()
        },
        CreateTask {
            body: Some("x".repeat(20_001)),
            ..base()
        },
        CreateTask {
            pool: Some(String::new()),
            ..base()
        },
        CreateTask {
            creator: "bad name".into(),
            ..base()
        },
        CreateTask {
            depends_on: vec!["nope".into()],
            ..base()
        },
        CreateTask {
            artifacts: vec![ArtifactInput {
                kind: ArtifactKind::File,
                path: "../etc".into(),
                commit: None,
                description: None,
            }],
            ..base()
        },
    ];
    for c in validation {
        assert!(
            matches!(hg.create_task(&c), Err(CorvusError::Validation(_))),
            "{c:?}"
        );
    }
    let missing_dep = CreateTask {
        depends_on: vec!["01890000-0000-7000-8000-000000000000".into()],
        ..base()
    };
    assert!(matches!(
        hg.create_task(&missing_dep),
        Err(CorvusError::NotFound(_))
    ));
    let missing_recipient = CreateTask {
        recipient: Some("ghost".into()),
        ..base()
    };
    assert!(matches!(
        hg.create_task(&missing_recipient),
        Err(CorvusError::NotFound(_))
    ));
    assert_eq!(count(&h.t.db, "tasks"), 0, "nothing half-created");
    assert_eq!(count(&h.t.db, "threads"), 0);
}

#[test]
fn create_idempotent_replay_returns_original_and_creates_nothing() {
    let h = H::new();
    let c = |title: &str| CreateTask {
        title: title.into(),
        idempotency_key: Some("k".into()),
        ..Default::default()
    };
    let a = h.create_with(c("first"));
    let events = h.events();
    let b = h.create_with(c("regenerated wording"));
    assert_eq!(a.id, b.id);
    assert_eq!(b.title, "first");
    assert_eq!(count(&h.t.db, "tasks"), 1);
    assert_eq!(count(&h.t.db, "threads"), 1);
    assert_eq!(h.events(), events);
    // Same key, other creator → separate task.
    let other = h.create_with(CreateTask {
        creator: "bob".into(),
        ..c("first")
    });
    assert_ne!(other.id, a.id);
}

#[test]
fn create_with_session_checks_owner_and_liveness() {
    let h = H::new();
    let s = h.session("alice");
    let ok = CreateTask {
        creator: "alice".into(),
        title: "t".into(),
        session_id: Some(s.clone()),
        ..Default::default()
    };
    h.s.huginn().create_task(&ok).unwrap();
    let wrong = CreateTask {
        creator: "bob".into(),
        ..ok.clone()
    };
    assert!(matches!(
        h.s.huginn().create_task(&wrong),
        Err(CorvusError::Conflict(_))
    ));
    h.s.sessions().end(&s).unwrap();
    assert_eq!(
        h.s.huginn().create_task(&ok).unwrap_err(),
        CorvusError::SessionNotActive
    );
}

// ---------- claim ----------

#[test]
fn claim_order_priority_then_oldest_then_insertion() {
    let h = H::new();
    h.create_with(CreateTask {
        title: "low-old".into(),
        priority: Some(1),
        ..Default::default()
    });
    h.create_with(CreateTask {
        title: "same-ms-a".into(),
        priority: Some(5),
        ..Default::default()
    });
    h.create_with(CreateTask {
        title: "same-ms-b".into(),
        priority: Some(5),
        ..Default::default()
    });
    h.t.clock.advance_secs(1);
    h.create_with(CreateTask {
        title: "high-new".into(),
        priority: Some(9),
        ..Default::default()
    });
    h.create_with(CreateTask {
        title: "neg".into(),
        priority: Some(-1),
        ..Default::default()
    });
    let order: Vec<_> = (0..5)
        .map(|i| h.claimed_title(&h.session(&format!("a{i}"))).unwrap())
        .collect();
    assert_eq!(order, ["high-new", "same-ms-a", "same-ms-b", "low-old", "neg"]);
    assert!(h.claim(&h.session("late")).task.is_none());
}

#[test]
fn claim_pool_filter_and_recipient_routing() {
    let h = H::new();
    let bob_session = h.session("bob");
    h.create_with(CreateTask {
        title: "for-bob".into(),
        recipient: Some("bob".into()),
        priority: Some(10),
        ..Default::default()
    });
    h.create_with(CreateTask {
        title: "ops".into(),
        pool: Some("ops".into()),
        ..Default::default()
    });
    h.create_with(CreateTask {
        title: "general".into(),
        ..Default::default()
    });
    let alice = h.session("alice");
    assert_eq!(h.claim_pool(&alice, Some("ops")).task.unwrap().title, "ops");
    let carol = h.session("carol");
    assert_eq!(
        h.claimed_title(&carol).as_deref(),
        Some("general"),
        "routed task is skipped for others"
    );
    let dave = h.session("dave");
    assert!(h.claim(&dave).task.is_none());
    assert_eq!(h.claimed_title(&bob_session).as_deref(), Some("for-bob"));
}

#[test]
fn claim_is_idempotent_per_session() {
    let h = H::new();
    h.create("a");
    h.create("b");
    let s = h.session("alice");
    let first = h.claim(&s);
    let events = h.events();
    let again = h.claim(&s);
    assert_eq!(first.claim.as_ref().unwrap().id, again.claim.as_ref().unwrap().id);
    assert_eq!(h.events(), events, "returning the existing claim writes nothing");
    assert_eq!(query_i64(&h.t.db, "SELECT count(*) FROM claims"), Some(1));
    let c = first.claim.unwrap();
    assert_eq!(c.lease_expires_at.0, h.t.now() + 900_000);
    assert_eq!(c.agent_name, "alice");
}

// ---------- dependencies ----------

#[test]
fn dependencies_block_until_done() {
    let h = H::new();
    let dep = h.create("dep");
    let child = h.create_with(CreateTask {
        title: "child".into(),
        priority: Some(9),
        depends_on: vec![dep.id.clone(), dep.id.clone()],
        ..Default::default()
    });
    assert_eq!(child.depends_on, vec![dep.id.clone()], "duplicates collapse");
    assert!(child.blocked && !child.blocked_by_cancelled);
    let s1 = h.session("a");
    assert_eq!(
        h.claimed_title(&s1).as_deref(),
        Some("dep"),
        "child is higher priority but blocked"
    );
    let s2 = h.session("b");
    assert!(h.claim(&s2).task.is_none());
    h.complete(&dep.id, &s1).unwrap();
    assert!(!h.s.huginn().get_task(&child.id).unwrap().blocked);
    assert_eq!(h.claimed_title(&s2).as_deref(), Some("child"));
}

#[test]
fn cancelled_dependency_keeps_dependent_blocked_and_flagged() {
    let h = H::new();
    let dep = h.create("dep");
    let child = h.create_with(CreateTask {
        title: "child".into(),
        depends_on: vec![dep.id.clone()],
        ..Default::default()
    });
    let r = h.cancel(&dep.id, false).unwrap();
    assert_eq!(r.cancelled, vec![dep.id.clone()]);
    let c = h.s.huginn().get_task(&child.id).unwrap();
    assert_eq!(
        c.status,
        TaskStatus::Open,
        "never silently cancelled without cascade"
    );
    assert!(c.blocked && c.blocked_by_cancelled);
    assert!(
        h.claim(&h.session("a")).task.is_none(),
        "never silently unblocked"
    );
}

// ---------- leases ----------

#[test]
fn lease_expires_exactly_at_the_boundary_and_is_reaped_by_claim() {
    let h = H::new();
    let task = h.create("x");
    let a = h.session("alice");
    h.claim(&a);
    let b = h.session("bob");
    h.t.clock.advance_ms(900_000 - 1);
    assert!(
        h.claim(&b).task.is_none(),
        "one millisecond before expiry the lease holds"
    );
    h.t.clock.advance_ms(1);
    let won = h.claim(&b);
    assert_eq!(
        won.task.unwrap().id,
        task.id,
        "at the boundary it is reaped and reclaimed"
    );
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM claims WHERE end_reason = 'expired' AND session_id IN (SELECT id FROM sessions WHERE agent_name = 'alice')"
        ),
        Some(1)
    );
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM events WHERE kind = 'lease.expired'"
        ),
        Some(1)
    );
}

#[test]
fn expire_leases_standalone() {
    let h = H::new();
    h.create("a");
    h.create("b");
    let (s1, s2) = (h.session("x"), h.session("y"));
    h.claim(&s1);
    h.claim(&s2);
    assert_eq!(h.s.huginn().expire_leases().unwrap().expired, 0);
    h.t.clock.advance_secs(900);
    assert_eq!(h.s.huginn().expire_leases().unwrap().expired, 2);
    assert_eq!(h.s.huginn().expire_leases().unwrap().expired, 0);
    let open =
        h.s.huginn()
            .list_tasks(&ListTasks {
                status: Some(TaskStatus::Open),
                ..Default::default()
            })
            .unwrap();
    assert_eq!(open.len(), 2);
}

#[test]
fn renew_by_holder_extends_other_conflicts_expired_conflicts() {
    let h = H::new();
    let task = h.create("x");
    let (a, b) = (h.session("alice"), h.session("bob"));
    h.claim(&a);
    h.t.clock.advance_secs(600);
    let r = h.s.huginn().renew_lease(&task.id, &a).unwrap();
    assert_eq!(r.holder.unwrap().lease_expires_at.0, h.t.now() + 900_000);
    assert!(
        matches!(h.s.huginn().renew_lease(&task.id, &b), Err(CorvusError::Conflict(m)) if m.contains("another session"))
    );
    // Expired but not reaped: renewal still wins (defaults #15).
    h.t.clock.advance_secs(900);
    h.s.huginn().renew_lease(&task.id, &a).unwrap();
    // Expired and reaped: conflict.
    h.t.clock.advance_secs(900);
    h.s.huginn().expire_leases().unwrap();
    let e = h.s.huginn().renew_lease(&task.id, &a).unwrap_err();
    assert!(
        matches!(&e, CorvusError::Conflict(m) if m.contains("expired")),
        "{e}"
    );
}

#[test]
fn release_reopens_and_replays_are_noops() {
    let h = H::new();
    let task = h.create("x");
    let (a, b) = (h.session("alice"), h.session("bob"));
    h.claim(&a);
    assert!(matches!(
        h.s.huginn().release_task(&task.id, &b),
        Err(CorvusError::Conflict(_))
    ));
    let r = h.s.huginn().release_task(&task.id, &a).unwrap();
    assert_eq!(r.status, TaskStatus::Open);
    let events = h.events();
    h.s.huginn().release_task(&task.id, &a).unwrap();
    assert_eq!(h.events(), events, "replay writes nothing");
    assert!(matches!(
        h.s.huginn().release_task(&task.id, &b),
        Err(CorvusError::Conflict(_))
    ));
}

#[test]
fn complete_is_final_and_replays_are_noops() {
    let h = H::new();
    let task = h.create("x");
    let (a, b) = (h.session("alice"), h.session("bob"));
    h.claim(&a);
    assert!(matches!(h.complete(&task.id, &b), Err(CorvusError::Conflict(_))));
    let done = h.complete(&task.id, &a).unwrap();
    assert_eq!(done.status, TaskStatus::Done);
    assert_eq!(done.result_summary.as_deref(), Some("shipped"));
    assert_eq!(done.completed_at.unwrap().0, h.t.now());
    let events = h.events();
    assert_eq!(h.complete(&task.id, &a).unwrap().status, TaskStatus::Done);
    assert_eq!(h.events(), events);
    assert!(matches!(
        h.s.huginn().release_task(&task.id, &a),
        Err(CorvusError::Conflict(_))
    ));
    assert!(matches!(
        h.s.huginn().renew_lease(&task.id, &a),
        Err(CorvusError::Conflict(_))
    ));
    assert!(h.claim(&b).task.is_none(), "done is final");
    let too_long = CompleteTask {
        session_id: a.clone(),
        result_summary: Some("x".repeat(20_001)),
    };
    assert!(matches!(
        h.s.huginn().complete_task(&task.id, &too_long),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn lease_ops_on_unknown_task_or_session() {
    let h = H::new();
    let a = h.session("alice");
    let ghost = "01890000-0000-7000-8000-000000000000";
    assert!(matches!(
        h.s.huginn().renew_lease(ghost, &a),
        Err(CorvusError::NotFound(_))
    ));
    let task = h.create("x");
    assert_eq!(
        h.s.huginn().renew_lease(&task.id, ghost).unwrap_err(),
        CorvusError::SessionNotActive
    );
}

// ---------- cancel ----------

#[test]
fn cancel_claimed_task_notifies_claimant_and_blocks_its_next_moves() {
    let h = H::new();
    let task = h.create("x");
    let a = h.session("alice");
    h.claim(&a);
    let r = h.cancel(&task.id, false).unwrap();
    assert_eq!(r.task.status, TaskStatus::Cancelled);
    assert_eq!(r.task.cancel_reason.as_deref(), Some("scope cut"));
    assert_eq!(r.task.cancelled_by.as_deref(), Some("op"));
    assert!(r.task.holder.is_none());
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM claims WHERE end_reason = 'cancelled'"
        ),
        Some(1)
    );
    assert_eq!(
        query_i64(
            &h.t.db,
            &format!(
            "SELECT count(*) FROM messages WHERE type = 'status' AND recipient = 'alice' AND sender = 'op'
               AND thread_id = '{}'", r.task.thread_id.unwrap())
        ),
        Some(1)
    );
    for e in [
        h.s.huginn().renew_lease(&task.id, &a).unwrap_err(),
        h.complete(&task.id, &a).unwrap_err(),
    ] {
        assert!(
            matches!(&e, CorvusError::Conflict(m) if m.contains("cancelled: scope cut")),
            "{e}"
        );
    }
    assert_eq!(h.s.huginn().claimant_of(&task.id).unwrap(), None);
}

#[test]
fn cancel_open_task_sends_no_message_and_is_idempotent() {
    let h = H::new();
    let task = h.create("x");
    let r = h.cancel(&task.id, false).unwrap();
    assert_eq!(r.cancelled.len(), 1);
    assert_eq!(count(&h.t.db, "messages"), 0);
    let events = h.events();
    let again = h.cancel(&task.id, false).unwrap();
    assert!(again.cancelled.is_empty());
    assert_eq!(h.events(), events);
}

#[test]
fn cancel_done_task_conflicts() {
    let h = H::new();
    let task = h.create("x");
    let a = h.session("a");
    h.claim(&a);
    h.complete(&task.id, &a).unwrap();
    assert!(matches!(h.cancel(&task.id, false), Err(CorvusError::Conflict(m)) if m.contains("done")));
}

#[test]
fn cascade_cancels_the_tree_and_skips_done_dependents() {
    let h = H::new();
    let dep = |title: &str, on: &[&TaskView]| {
        h.create_with(CreateTask {
            title: title.into(),
            depends_on: on.iter().map(|t| t.id.clone()).collect(),
            ..Default::default()
        })
    };
    let root = h.create("root");
    let mid = dep("mid", &[&root]);
    let leaf = dep("leaf", &[&mid]);
    let diamond = dep("diamond", &[&root, &mid]);
    let finished = dep("finished", &[&mid]);
    let unrelated = h.create("unrelated");
    // Through the services a dependent cannot finish before its dependencies, so a done dependent of a live
    // root only arises from data the operator changed by hand; force it to prove the skip.
    corvus_core::testing::exec(
        &h.t.db,
        &format!("UPDATE tasks SET status = 'done' WHERE id = '{}'", finished.id),
    )
    .unwrap();
    // A claimed dependent: its claimant is notified too.
    corvus_core::testing::exec(
        &h.t.db,
        &format!("UPDATE tasks SET status = 'claimed' WHERE id = '{}'", leaf.id),
    )
    .unwrap();
    let w = h.session("worker");
    corvus_core::testing::exec(
        &h.t.db,
        &format!(
            "INSERT INTO claims (id, task_id, session_id, agent_name, claimed_at, lease_expires_at)
             VALUES ('c-leaf', '{}', '{w}', 'worker', 1, {})",
            leaf.id,
            h.t.now() + 900_000
        ),
    )
    .unwrap();

    let before = h.events();
    let r = h.cancel(&root.id, true).unwrap();
    let mut got = r.cancelled.clone();
    got.sort();
    let mut want = vec![
        root.id.clone(),
        mid.id.clone(),
        leaf.id.clone(),
        diamond.id.clone(),
    ];
    want.sort();
    assert_eq!(got, want, "each dependent once, done ones skipped");
    assert_eq!(r.cancelled[0], root.id, "the root first");
    assert_eq!(h.events(), before + 4, "one event per cancelled task");
    assert_eq!(
        h.s.huginn().get_task(&finished.id).unwrap().status,
        TaskStatus::Done
    );
    assert_eq!(
        h.s.huginn().get_task(&unrelated.id).unwrap().status,
        TaskStatus::Open
    );
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM claims WHERE id = 'c-leaf' AND end_reason = 'cancelled'"
        ),
        Some(1)
    );
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM messages WHERE recipient = 'worker' AND type = 'status'"
        ),
        Some(1)
    );
}

#[test]
fn cancel_validation() {
    let h = H::new();
    let task = h.create("x");
    let hg = h.s.huginn();
    let ghost = "01890000-0000-7000-8000-000000000000";
    let by = |b: &str| CancelTask {
        by: b.into(),
        ..Default::default()
    };
    assert!(matches!(
        hg.cancel_task(ghost, &by("op")),
        Err(CorvusError::NotFound(_))
    ));
    assert!(matches!(
        hg.cancel_task(&task.id, &by("not ok")),
        Err(CorvusError::Validation(_))
    ));
    let long = CancelTask {
        by: "op".into(),
        reason: Some("r".repeat(2_001)),
        ..Default::default()
    };
    assert!(matches!(
        hg.cancel_task(&task.id, &long),
        Err(CorvusError::Validation(_))
    ));
}

// ---------- append, claimant, list ----------

#[test]
fn append_only_while_open_and_truncated_at_cap() {
    let h = H::new();
    let task = h.create_with(CreateTask {
        title: "x".into(),
        body: Some("start".into()),
        ..Default::default()
    });
    let t = h.s.huginn().append_to_task(&task.id, "more").unwrap();
    assert_eq!(t.body, "start\n\nmore");
    let t =
        h.s.huginn()
            .append_to_task(&task.id, &"y".repeat(25_000))
            .unwrap();
    assert_eq!(t.body.chars().count(), 20_000);
    assert_eq!(
        query_i64(
            &h.t.db,
            "SELECT count(*) FROM events WHERE kind = 'task.appended'"
        ),
        Some(2)
    );
    let a = h.session("a");
    h.claim(&a);
    assert!(matches!(
        h.s.huginn().append_to_task(&task.id, "late"),
        Err(CorvusError::Conflict(_))
    ));
    assert!(matches!(
        h.s.huginn().append_to_task(&task.id, "  "),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn claimant_of_reports_holder() {
    let h = H::new();
    let task = h.create("x");
    assert_eq!(h.s.huginn().claimant_of(&task.id).unwrap(), None);
    let a = h.session("alice");
    h.claim(&a);
    let holder = h.s.huginn().claimant_of(&task.id).unwrap().unwrap();
    assert_eq!(
        (holder.agent_name.as_str(), holder.session_id.as_str()),
        ("alice", a.as_str())
    );
    assert!(matches!(
        h.s.huginn().claimant_of("01890000-0000-7000-8000-000000000000"),
        Err(CorvusError::NotFound(_))
    ));
}

#[test]
fn list_tasks_filters_by_status_agent_and_limit() {
    let h = H::new();
    h.create_with(CreateTask {
        creator: "alice".into(),
        title: "by-alice".into(),
        ..Default::default()
    });
    h.t.clock.advance_ms(1);
    h.create_with(CreateTask {
        title: "for-bob".into(),
        recipient: Some("alice".into()),
        ..Default::default()
    });
    h.t.clock.advance_ms(1);
    h.create("plain");
    let carol = h.session("carol");
    h.t.clock.advance_ms(1);
    let held = h.claim(&carol).task.unwrap();
    let hg = h.s.huginn();
    let all = hg.list_tasks(&ListTasks::default()).unwrap();
    assert_eq!(
        all.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        ["plain", "for-bob", "by-alice"]
    );
    let alice = hg
        .list_tasks(&ListTasks {
            agent: Some("alice".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(alice.len(), 2, "creator or recipient");
    let carols = hg
        .list_tasks(&ListTasks {
            agent: Some("carol".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        carols.iter().map(|t| &t.id).collect::<Vec<_>>(),
        [&held.id],
        "holder"
    );
    let claimed = hg
        .list_tasks(&ListTasks {
            status: Some(TaskStatus::Claimed),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].holder.as_ref().unwrap().agent_name, "carol");
    assert_eq!(
        hg.list_tasks(&ListTasks {
            limit: Some(1),
            ..Default::default()
        })
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn claim_history_rows_record_each_end_reason() {
    let h = H::new();
    let a = h.session("a");
    let t1 = h.create("released");
    h.claim(&a);
    h.s.huginn().release_task(&t1.id, &a).unwrap();
    h.cancel(&t1.id, false).unwrap();
    let t2 = h.create("completed");
    h.claim(&a);
    h.complete(&t2.id, &a).unwrap();
    let t3 = h.create("expired");
    h.claim(&a);
    h.t.clock.advance_secs(900);
    h.s.huginn().expire_leases().unwrap();
    h.cancel(&t3.id, false).unwrap(); // reopened by expiry; take it out of the way (no claim, so no claim row)
    let t4 = h.create("cancelled");
    h.claim(&a);
    h.cancel(&t4.id, false).unwrap();
    for r in EndReason::ALL {
        assert_eq!(
            query_i64(
                &h.t.db,
                &format!("SELECT count(*) FROM claims WHERE end_reason = '{}'", r.as_str())
            ),
            Some(1),
            "{r}"
        );
    }
}

#[test]
fn every_mutation_writes_exactly_one_event_without_bodies() {
    const SECRET: &str = "BODY-TEXT-MUST-NOT-LEAK";
    let h = H::new();
    let hg = h.s.huginn();
    let step = |name: &str, f: &dyn Fn()| {
        let before = h.events();
        f();
        assert_eq!(h.events(), before + 1, "{name} must write exactly one event");
    };
    let session = std::cell::RefCell::new(String::new());
    step("register", &|| *session.borrow_mut() = h.session("alice"));
    let s = session.borrow().clone();
    let task = std::cell::RefCell::new(String::new());
    step("create", &|| {
        *task.borrow_mut() = h
            .create_with(CreateTask {
                title: "t".into(),
                body: Some(SECRET.into()),
                ..Default::default()
            })
            .id
    });
    let t = task.borrow().clone();
    step("append", &|| {
        hg.append_to_task(&t, SECRET).unwrap();
    });
    step("claim", &|| {
        h.claim(&s);
    });
    step("renew", &|| {
        hg.renew_lease(&t, &s).unwrap();
    });
    step("release", &|| {
        hg.release_task(&t, &s).unwrap();
    });
    step("claim again", &|| {
        h.claim(&s);
    });
    step("complete", &|| {
        hg.complete_task(
            &t,
            &CompleteTask {
                session_id: s.clone(),
                result_summary: Some(SECRET.into()),
            },
        )
        .unwrap();
    });
    let t2 = h.create("t2").id;
    step("claim t2", &|| {
        h.claim(&s);
    });
    step("cancel (claimed)", &|| {
        h.cancel(&t2, false).unwrap();
    });
    let t3 = h.create("t3").id;
    let s2 = h.session("bob");
    h.claim(&s2);
    h.t.clock.advance_secs(900);
    step("expire", &|| {
        assert_eq!(hg.expire_leases().unwrap().expired, 1);
    });
    let _ = t3;
    step("end session", &|| {
        h.s.sessions().end(&s).unwrap();
    });
    assert_eq!(
        query_i64(
            &h.t.db,
            &format!("SELECT count(*) FROM events WHERE summary LIKE '%{SECRET}%'")
        ),
        Some(0)
    );
}
