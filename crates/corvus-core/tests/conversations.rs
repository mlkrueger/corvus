//! Conversation links: an agent's identity follows its tool conversation (D0023, system/03).
#![allow(clippy::unwrap_used)]

use corvus_core::Services;
use corvus_core::api::{ConversationInput, RegisterSession, SessionView};
use corvus_core::testing::{TestDb, count, exec, query_i64};
use corvus_domain::{ConversationRef, CorvusError, Tool};

const CONV_A: &str = "48ab6180-bde1-4698-b7a6-ac1a9e20db89";
const CONV_B: &str = "6b9d1bc1-bfe4-4dd2-b6f8-03ed95f1def4";

fn reg(s: &Services, name: Option<&str>, tool: &str, id: &str) -> Result<SessionView, CorvusError> {
    s.sessions().register(&RegisterSession {
        agent_name: name.map(str::to_owned),
        client_name: "test".into(),
        conversation: Some(ConversationInput {
            tool: tool.into(),
            id: id.into(),
            directory: Some("/work/project".into()),
        }),
        ..RegisterSession::default()
    })
}

fn claude(id: &str) -> ConversationRef {
    ConversationRef::new(Tool::ClaudeCode, id).unwrap()
}

fn events(t: &TestDb, kind: &str) -> i64 {
    query_i64(
        &t.db,
        &format!("SELECT count(*) FROM events WHERE kind = '{kind}'"),
    )
    .unwrap()
}

#[test]
fn an_unknown_conversation_is_linked_to_the_agent_registered_with_it() {
    let t = TestDb::new();
    let s = t.services();
    let first = reg(&s, None, "claude-code", CONV_A).unwrap();
    assert!(first.name_generated);
    assert_eq!(
        s.sessions()
            .agent_for_conversation(&claude(CONV_A))
            .unwrap()
            .unwrap()
            .as_str(),
        first.agent_name
    );
    assert_eq!(count(&t.db, "conversations"), 1);
    assert_eq!(events(&t, "conversation.linked"), 1);

    let links = s.sessions().conversations_for_agent(&first.agent_name).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].tool, "claude-code");
    assert_eq!(links[0].conversation_id, CONV_A);
    assert_eq!(links[0].directory.as_deref(), Some("/work/project"));
    assert_eq!(links[0].carried_from, None);
}

#[test]
fn a_resumed_conversation_gets_the_same_agent_and_a_new_session() {
    let t = TestDb::new();
    let s = t.services();
    let first = reg(&s, None, "claude-code", CONV_A).unwrap();
    s.sessions().end(&first.id).unwrap();

    let resumed = reg(&s, None, "claude-code", CONV_A).unwrap();
    assert_eq!(resumed.agent_name, first.agent_name);
    assert_ne!(resumed.id, first.id);
    assert_eq!(count(&t.db, "conversations"), 1, "no second link");
    assert_eq!(events(&t, "conversation.linked"), 1);
    assert_eq!(count(&t.db, "agents"), 1, "no second agent was generated");
}

#[test]
fn a_linked_conversation_ignores_a_different_requested_name() {
    let t = TestDb::new();
    let s = t.services();
    reg(&s, Some("alpha"), "claude-code", CONV_A).unwrap();
    let again = reg(&s, Some("beta"), "claude-code", CONV_A).unwrap();
    assert_eq!(again.agent_name, "alpha");
    assert_eq!(count(&t.db, "agents"), 1, "beta was never created");
}

#[test]
fn two_live_sessions_may_share_one_conversation() {
    let t = TestDb::new();
    let s = t.services();
    let a = reg(&s, Some("alpha"), "claude-code", CONV_A).unwrap();
    let b = reg(&s, None, "claude-code", CONV_A).unwrap();
    assert_eq!((a.agent_name.as_str(), b.agent_name.as_str()), ("alpha", "alpha"));
    assert_ne!(a.id, b.id);
}

#[test]
fn the_same_id_under_another_tool_is_a_different_conversation() {
    let t = TestDb::new();
    let s = t.services();
    let a = reg(&s, None, "claude-code", "shared-id").unwrap();
    let b = reg(&s, None, "opencode", "shared-id").unwrap();
    assert_ne!(a.agent_name, b.agent_name);
    assert_eq!(count(&t.db, "conversations"), 2);
}

#[test]
fn registering_without_a_conversation_links_nothing() {
    let t = TestDb::new();
    let s = t.services();
    s.sessions()
        .register(&RegisterSession {
            client_name: "test".into(),
            ..RegisterSession::default()
        })
        .unwrap();
    assert_eq!(count(&t.db, "conversations"), 0);
    assert_eq!(
        s.sessions().agent_for_conversation(&claude(CONV_A)).unwrap(),
        None
    );
}

#[test]
fn bad_conversation_input_is_a_validation_error_and_writes_nothing() {
    let t = TestDb::new();
    let s = t.services();
    for (tool, id) in [
        ("claude-code", ""),
        ("claude-code", "../etc"),
        ("claude-code", "a b"),
        ("vim", CONV_A),
        ("", CONV_A),
    ] {
        let err = reg(&s, Some("alpha"), tool, id).unwrap_err();
        assert!(
            matches!(err, CorvusError::Validation(_)),
            "{tool:?} {id:?}: {err}"
        );
    }
    assert_eq!(count(&t.db, "sessions"), 0);
    assert_eq!(count(&t.db, "conversations"), 0);
}

#[test]
fn an_unusable_directory_is_dropped_not_rejected() {
    let t = TestDb::new();
    let s = t.services();
    let out = s
        .sessions()
        .register(&RegisterSession {
            client_name: "test".into(),
            conversation: Some(ConversationInput {
                tool: "opencode".into(),
                id: "ses_abc12345".into(),
                directory: Some("x".repeat(2000)),
            }),
            ..RegisterSession::default()
        })
        .unwrap();
    let links = s.sessions().conversations_for_agent(&out.agent_name).unwrap();
    assert_eq!(links[0].directory, None);
}

#[test]
fn a_clear_carries_the_agent_to_the_new_id() {
    let t = TestDb::new();
    let s = t.services();
    reg(&s, Some("alpha"), "claude-code", CONV_A).unwrap();

    let carried = s
        .sessions()
        .carry_conversation(&claude(CONV_A), &claude(CONV_B))
        .unwrap();
    assert_eq!(carried.unwrap().as_str(), "alpha");
    assert_eq!(
        s.sessions()
            .agent_for_conversation(&claude(CONV_B))
            .unwrap()
            .unwrap()
            .as_str(),
        "alpha"
    );
    assert_eq!(events(&t, "conversation.carried"), 1);

    // Newest first, and the old id still resolves (resuming the pre-clear conversation works).
    let links = s.sessions().conversations_for_agent("alpha").unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].conversation_id, CONV_B);
    assert_eq!(links[0].carried_from.as_deref(), Some(CONV_A));
    assert_eq!(links[0].directory.as_deref(), Some("/work/project"));
    assert_eq!(
        s.sessions()
            .agent_for_conversation(&claude(CONV_A))
            .unwrap()
            .unwrap()
            .as_str(),
        "alpha"
    );

    // Registering under the new id is the same agent.
    assert_eq!(reg(&s, None, "claude-code", CONV_B).unwrap().agent_name, "alpha");
}

#[test]
fn carry_is_a_no_op_when_there_is_nothing_to_carry_or_the_target_is_taken() {
    let t = TestDb::new();
    let s = t.services();
    // `from` is not linked.
    assert_eq!(
        s.sessions()
            .carry_conversation(&claude(CONV_A), &claude(CONV_B))
            .unwrap(),
        None
    );
    assert_eq!(count(&t.db, "conversations"), 0);

    // `to` already belongs to someone else: unchanged.
    reg(&s, Some("alpha"), "claude-code", CONV_A).unwrap();
    reg(&s, Some("beta"), "claude-code", CONV_B).unwrap();
    assert_eq!(
        s.sessions()
            .carry_conversation(&claude(CONV_A), &claude(CONV_B))
            .unwrap(),
        None
    );
    assert_eq!(
        s.sessions()
            .agent_for_conversation(&claude(CONV_B))
            .unwrap()
            .unwrap()
            .as_str(),
        "beta"
    );
    assert_eq!(events(&t, "conversation.carried"), 0);

    // Across tools is refused.
    let other = ConversationRef::new(Tool::OpenCode, "ses_abc12345").unwrap();
    assert!(matches!(
        s.sessions().carry_conversation(&claude(CONV_A), &other),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn links_are_immutable_in_the_store() {
    let t = TestDb::new();
    let s = t.services();
    reg(&s, Some("alpha"), "claude-code", CONV_A).unwrap();
    reg(&s, Some("beta"), "claude-code", CONV_B).unwrap();
    assert!(exec(&t.db, "UPDATE conversations SET agent_name = 'beta'").is_err());
    assert!(exec(&t.db, "DELETE FROM conversations").is_err());
    assert_eq!(count(&t.db, "conversations"), 2);
}

#[test]
fn the_store_rejects_ids_the_domain_would_reject() {
    let t = TestDb::new();
    reg(&t.services(), Some("alpha"), "claude-code", CONV_A).unwrap();
    for (tool, id) in [("claude-code", "a/b"), ("claude-code", ""), ("Bad Tool", "x")] {
        let sql = format!(
            "INSERT INTO conversations (tool, conversation_id, agent_name, created_at) VALUES ('{tool}', '{id}', 'alpha', 1)"
        );
        assert!(exec(&t.db, &sql).is_err(), "{tool:?} {id:?}");
    }
    assert!(
        exec(
            &t.db,
            "INSERT INTO conversations (tool, conversation_id, agent_name, created_at) VALUES ('codex', 'x', 'nobody', 1)"
        )
        .is_err(),
        "unknown agent"
    );
}

#[test]
fn the_link_and_the_session_commit_together() {
    let t = TestDb::new();
    let s = t.services();
    let out = reg(&s, None, "claude-code", CONV_A).unwrap();
    // One registration: one session, one link, both events carrying the same session id.
    for kind in ["session.registered", "conversation.linked"] {
        let n = query_i64(
            &t.db,
            &format!(
                "SELECT count(*) FROM events WHERE kind = '{kind}' AND session_id = '{}'",
                out.id
            ),
        );
        assert_eq!(n, Some(1), "{kind}");
    }
}
