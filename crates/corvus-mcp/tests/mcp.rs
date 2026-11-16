//! The MCP server driven over an in-memory pipe with hand-rolled JSON-RPC (newline-delimited), against real
//! SQLite files.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use corvus_core::Corvus;
use corvus_core::hermod::Hermod;
use corvus_core::testing::{TestDb, count, exec};
use corvus_domain::{AgentName, CorvusError, GUIDANCE};
use corvus_mcp::McpOptions;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf, split};
use tokio::task::JoinHandle;

const TOOLS: [&str; 20] = [
    "register_session",
    "get_bootstrap_context",
    "create_task",
    "claim_task",
    "renew_lease",
    "release_task",
    "complete_task",
    "cancel_task",
    "post_message",
    "create_thread",
    "send_direct_message",
    "read_inbox",
    "wait_for_inbox",
    "ack_message",
    "heartbeat",
    "end_session",
    "write_digest",
    "record_decision",
    "list_decisions",
    "export_state_to_git",
];

struct Client {
    w: Option<WriteHalf<DuplexStream>>,
    r: Lines<BufReader<ReadHalf<DuplexStream>>>,
    next: u64,
    server: JoinHandle<Result<(), CorvusError>>,
}

fn corvus(t: &TestDb) -> Corvus {
    Corvus::new(t.services(), Hermod::new(&t.path))
}

impl Client {
    /// Starts a server on `t`'s database and completes the initialize handshake.
    async fn start(t: &TestDb, default_agent: Option<&str>) -> (Self, Value) {
        let options = McpOptions {
            default_agent: default_agent.map(|a| AgentName::parse(a).unwrap()),
            ..McpOptions::default()
        };
        Self::start_as(t, options, "t").await
    }

    /// [`Client::start`] with explicit host options and the `clientInfo.name` sent at `initialize`.
    async fn start_as(t: &TestDb, options: McpOptions, client_name: &str) -> (Self, Value) {
        let (c_to_s, s_in) = tokio::io::duplex(1 << 20);
        let (s_out, s_to_c) = tokio::io::duplex(1 << 20);
        let server = tokio::spawn(corvus_mcp::run(corvus(t), options, s_in, s_out));
        let (_unused, w) = split(c_to_s);
        let (r, _unused2) = split(s_to_c);
        let mut c = Client {
            w: Some(w),
            r: BufReader::new(r).lines(),
            next: 1,
            server,
        };
        let init = c
            .request(
                "initialize",
                json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": client_name, "version": "0"}}),
            )
            .await;
        c.write(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        (c, init)
    }

    async fn write(&mut self, v: &Value) {
        let w = self.w.as_mut().expect("client closed");
        w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        w.flush().await.unwrap();
    }

    async fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        id
    }

    async fn recv(&mut self, id: u64) -> Value {
        loop {
            let line = tokio::time::timeout(Duration::from_secs(30), self.r.next_line())
                .await
                .expect("timed out waiting for the server")
                .unwrap()
                .expect("server closed the pipe");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") == Some(&json!(id)) {
                assert!(v.get("error").is_none(), "protocol error: {v}");
                return v["result"].clone();
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params).await;
        self.recv(id).await
    }

    async fn send_tool(&mut self, name: &str, args: Value) -> u64 {
        self.send("tools/call", json!({"name": name, "arguments": args}))
            .await
    }

    /// A tool call carrying request `_meta`, as the agent tools send it.
    async fn tool_meta(&mut self, name: &str, args: Value, meta: Value) -> (bool, Value) {
        let id = self
            .send(
                "tools/call",
                json!({"name": name, "arguments": args, "_meta": meta}),
            )
            .await;
        self.finish_tool(id).await
    }

    /// `(is_error, parsed JSON text)`.
    async fn finish_tool(&mut self, id: u64) -> (bool, Value) {
        let r = self.recv(id).await;
        let text = r["content"][0]["text"].as_str().expect("text content");
        (r["isError"] == json!(true), serde_json::from_str(text).unwrap())
    }

    async fn tool(&mut self, name: &str, args: Value) -> (bool, Value) {
        let id = self.send_tool(name, args).await;
        self.finish_tool(id).await
    }

    async fn ok(&mut self, name: &str, args: Value) -> Value {
        let (err, v) = self.tool(name, args).await;
        assert!(!err, "{name} failed: {v}");
        v
    }

    async fn close(mut self) -> Result<(), CorvusError> {
        drop(self.w.take());
        tokio::time::timeout(Duration::from_secs(10), self.server)
            .await
            .expect("server did not exit on EOF")
            .unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_instructions_and_tool_list() {
    let t = TestDb::new();
    let (mut c, init) = Client::start(&t, None).await;
    assert_eq!(init["instructions"], GUIDANCE);
    assert_eq!(init["serverInfo"]["name"], "corvus");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let list = c.request("tools/list", json!({})).await;
    let tools = list["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    let mut want = TOOLS.to_vec();
    want.sort_unstable();
    assert_eq!(names, want);
    for tool in tools {
        assert!(tool["inputSchema"].is_object(), "{tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
    // Schemas come from the core request structs; the fields the process fills in are not required.
    let by = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap()["inputSchema"].clone();
    let create = by("create_task");
    assert!(create["properties"]["title"].is_object());
    let required = create["required"].as_array().unwrap();
    assert!(required.contains(&json!("title")));
    assert!(!required.contains(&json!("creator")) && !required.contains(&json!("session_id")));
    assert!(
        by("complete_task")["required"]
            .as_array()
            .unwrap()
            .contains(&json!("task_id"))
    );
    assert!(by("ack_message")["properties"]["message_id"]["type"] == "integer");
    c.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn full_flow_with_session_convenience() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, None).await;
    let (mut b, _) = Client::start(&t, None).await;

    let reg = a
        .ok(
            "register_session",
            json!({"agent_name": "alpha", "client_name": "test"}),
        )
        .await;
    assert_eq!(reg["session"]["agent_name"], "alpha");
    assert!(reg["bootstrap"].is_object() && reg["guidance"] == GUIDANCE);
    let sid = reg["session"]["id"].as_str().unwrap().to_owned();
    b.ok(
        "register_session",
        json!({"agent_name": "beta", "client_name": "test"}),
    )
    .await;

    // creator and session_id omitted: filled in.
    let task = a.ok("create_task", json!({"title": "do it"})).await;
    assert_eq!(task["creator"], "alpha");
    let tid = task["id"].as_str().unwrap().to_owned();
    let claim = a.ok("claim_task", json!({})).await;
    assert_eq!(claim["task"]["id"], tid.as_str());
    assert_eq!(claim["claim"]["session_id"], sid.as_str());
    a.ok("renew_lease", json!({"task_id": tid})).await;
    let done = a
        .ok("complete_task", json!({"task_id": tid, "result_summary": "ok"}))
        .await;
    assert_eq!(done["status"], "done");

    let thread = a.ok("create_thread", json!({"title": "chat"})).await;
    let thread_id = thread["id"].as_str().unwrap().to_owned();
    let msg = a
        .ok(
            "post_message",
            json!({"thread_id": thread_id, "to": "beta", "body": "hello beta"}),
        )
        .await;
    assert_eq!(msg["sender"], "alpha");
    let direct = a
        .ok("send_direct_message", json!({"to": "beta", "body": "psst"}))
        .await;
    assert_eq!(direct["sender"], "alpha");

    let inbox = b.ok("read_inbox", json!({})).await;
    let bodies: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(
        bodies.contains(&"hello beta") && bodies.contains(&"psst"),
        "{inbox}"
    );
    let first = inbox[0]["id"].as_i64().unwrap();
    let acked = b.ok("ack_message", json!({"message_id": first})).await;
    assert_eq!(acked["message_id"], first);
    assert_eq!(acked["newly_acked"], true);
    assert_eq!(b.ok("read_inbox", json!({})).await.as_array().unwrap().len(), 1);

    a.ok("heartbeat", json!({})).await;
    let boot = a.ok("get_bootstrap_context", json!({})).await;
    assert_eq!(boot["agent"], "alpha");
    a.ok("write_digest", json!({"scope": "global", "summary": "s"}))
        .await;
    let dec = a
        .ok("record_decision", json!({"title": "t", "decision": "d"}))
        .await;
    assert_eq!(dec["decided_by"], "alpha");
    assert_eq!(
        a.ok("list_decisions", json!({})).await.as_array().unwrap().len(),
        1
    );

    let ended = a.ok("end_session", json!({})).await;
    assert!(ended["ended_at"].is_string());
    a.close().await.unwrap();
    b.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_are_tool_errors_with_the_json_shape() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, None).await;
    let (mut b, _) = Client::start(&t, None).await;
    a.ok(
        "register_session",
        json!({"agent_name": "alpha", "client_name": "test"}),
    )
    .await;
    b.ok(
        "register_session",
        json!({"agent_name": "beta", "client_name": "test"}),
    )
    .await;

    let task = a.ok("create_task", json!({"title": "x"})).await;
    let tid = task["id"].as_str().unwrap();
    a.ok("claim_task", json!({})).await;
    // beta does not hold alpha's task.
    let (is_err, v) = b.tool("complete_task", json!({"task_id": tid})).await;
    assert!(is_err);
    assert_eq!(v["error"]["code"], "conflict", "{v}");
    assert!(v["error"]["message"].is_string());

    // Unknown field and missing id: validation.
    let (is_err, v) = a.tool("claim_task", json!({"bogus": 1})).await;
    assert!(is_err && v["error"]["code"] == "validation", "{v}");
    let (is_err, v) = a.tool("complete_task", json!({})).await;
    assert!(is_err && v["error"]["code"] == "validation", "{v}");
    // Unknown task.
    let (is_err, v) = a
        .tool(
            "complete_task",
            json!({"task_id": "00000000-0000-4000-8000-000000000000"}),
        )
        .await;
    assert!(is_err && v["error"]["code"] == "not_found", "{v}");

    // After end_session the remembered session is dead.
    a.ok("end_session", json!({})).await;
    let (is_err, v) = a.tool("heartbeat", json!({})).await;
    assert!(is_err, "{v}");
    assert_eq!(v["error"]["code"], "session_not_active");
    let (is_err, v) = a.tool("create_task", json!({"title": "y"})).await;
    assert!(is_err && v["error"]["code"] == "session_not_active", "{v}");
    a.close().await.unwrap();
    b.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_server_says_restart_after_another_process_upgrades_the_schema() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, None).await;
    a.ok(
        "register_session",
        json!({"agent_name": "alpha", "client_name": "test"}),
    )
    .await;
    // Another process migrates the database underneath this server.
    let other = t.reopen();
    exec(&other, "PRAGMA user_version = 99").unwrap();
    let (is_err, v) = a.tool("create_task", json!({"title": "x"})).await;
    assert!(is_err, "{v}");
    assert_eq!(v["error"]["code"], "upgraded", "{v}");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains("restart this process"), "{message}");
    // Ending the session on close is a write, so it fails the same way; that is expected here.
    let _ = a.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_before_registering_need_explicit_arguments() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, None).await;
    let (is_err, v) = a.tool("create_task", json!({"title": "x"})).await;
    assert!(is_err && v["error"]["code"] == "validation", "{v}");
    let (is_err, v) = a.tool("no_such_tool", json!({})).await;
    assert!(is_err && v["error"]["code"] == "not_found", "{v}");
    a.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn default_agent_names_the_session() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, Some("from-env")).await;
    let reg = a.ok("register_session", json!({"client_name": "test"})).await;
    assert_eq!(reg["session"]["agent_name"], "from-env");
    assert_eq!(reg["session"]["name_generated"], false);
    // An explicit name still wins.
    let (mut b, _) = Client::start(&t, Some("from-env")).await;
    let reg = b
        .ok(
            "register_session",
            json!({"client_name": "test", "agent_name": "explicit"}),
        )
        .await;
    assert_eq!(reg["session"]["agent_name"], "explicit");
    a.close().await.unwrap();
    b.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn eof_ends_the_remembered_session() {
    let t = TestDb::new();
    let (mut a, _) = Client::start(&t, None).await;
    let reg = a
        .ok(
            "register_session",
            json!({"agent_name": "alpha", "client_name": "test"}),
        )
        .await;
    let sid = reg["session"]["id"].as_str().unwrap().to_owned();
    assert!(t.services().sessions().get(&sid).unwrap().ended_at.is_none());
    a.close().await.unwrap();
    let after = t.services().sessions().get(&sid).unwrap();
    assert!(after.ended_at.is_some(), "session should have ended on EOF");
}

#[tokio::test(flavor = "multi_thread")]
async fn eof_without_a_session_is_clean() {
    let t = TestDb::new();
    let (a, _) = Client::start(&t, None).await;
    a.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn two_servers_one_database_wait_wakes_on_post() {
    let t = TestDb::new();
    let (mut waiter, _) = Client::start(&t, None).await;
    let (mut poster, _) = Client::start(&t, None).await;
    waiter
        .ok(
            "register_session",
            json!({"agent_name": "waiter", "client_name": "test"}),
        )
        .await;
    poster
        .ok(
            "register_session",
            json!({"agent_name": "poster", "client_name": "test"}),
        )
        .await;

    let wait_id = waiter
        .send_tool("wait_for_inbox", json!({"timeout_seconds": 60}))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    poster
        .ok("send_direct_message", json!({"to": "waiter", "body": "wake up"}))
        .await;

    let (is_err, v) = tokio::time::timeout(Duration::from_secs(20), waiter.finish_tool(wait_id))
        .await
        .expect("waiter did not wake");
    assert!(!is_err, "{v}");
    assert_eq!(v[0]["body"], "wake up");
    waiter.close().await.unwrap();
    poster.close().await.unwrap();
}

// ---------- conversations (D0023) ----------

const CLAUDE_ID: &str = "48ab6180-bde1-4698-b7a6-ac1a9e20db89";

fn opencode(session: &str) -> Value {
    json!({"ai.opencode/sessionID": session, "progressToken": 1})
}

/// What Claude Code 2.1.325 sends with every request (it names itself here, not at `initialize`).
fn claude_meta() -> Value {
    json!({
        "io.modelcontextprotocol/clientInfo": {"name": "claude-code", "version": "2.1.325"},
        "claudecode/toolUseId": "toolu_x",
    })
}

fn claude_options() -> McpOptions {
    McpOptions {
        claude_session_id: Some(CLAUDE_ID.into()),
        directory: Some("/work/project".into()),
        ..McpOptions::default()
    }
}

fn links(t: &TestDb, agent: &str) -> Vec<corvus_core::api::ConversationView> {
    t.services().sessions().conversations_for_agent(agent).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_register_schema_does_not_offer_a_conversation_and_ignores_one() {
    let t = TestDb::new();
    let (mut c, _) = Client::start(&t, None).await;
    let tools = c.request("tools/list", json!({})).await;
    let register = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "register_session")
        .unwrap();
    assert!(
        register["inputSchema"]["properties"]
            .get("conversation")
            .is_none(),
        "{register}"
    );
    // A model that sends one anyway cannot choose who it is.
    let out = c
        .ok(
            "register_session",
            json!({"client_name": "test", "conversation": {"tool": "claude-code", "id": CLAUDE_ID}}),
        )
        .await;
    assert!(links(&t, out["session"]["agent_name"].as_str().unwrap()).is_empty());
    assert_eq!(count(&t.db, "conversations"), 0);
    c.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_code_is_linked_by_the_environment_id_and_a_resume_gets_the_same_agent() {
    let t = TestDb::new();
    let (mut a, _) = Client::start_as(&t, claude_options(), "t").await;
    let (err, out) = a
        .tool_meta(
            "register_session",
            json!({"client_name": "claude-code"}),
            claude_meta(),
        )
        .await;
    assert!(!err, "{out}");
    let agent = out["session"]["agent_name"].as_str().unwrap().to_owned();
    let l = links(&t, &agent);
    assert_eq!(l.len(), 1);
    assert_eq!(
        (l[0].tool.as_str(), l[0].conversation_id.as_str()),
        ("claude-code", CLAUDE_ID)
    );
    assert_eq!(l[0].directory.as_deref(), Some("/work/project"));
    // Later calls from the same conversation use the remembered session.
    let (err, hb) = a.tool_meta("heartbeat", json!({}), claude_meta()).await;
    assert!(!err, "{hb}");
    a.close().await.unwrap();

    // A new process on the same conversation (`claude --resume`): same agent, new session.
    let (mut b, _) = Client::start_as(&t, claude_options(), "t").await;
    let (_, again) = b
        .tool_meta(
            "register_session",
            json!({"client_name": "claude-code"}),
            claude_meta(),
        )
        .await;
    assert_eq!(again["session"]["agent_name"], json!(agent));
    assert_ne!(again["session"]["id"], out["session"]["id"]);
    assert_eq!(count(&t.db, "conversations"), 1);
    b.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claude_id_leaked_into_another_tools_server_is_ignored() {
    let t = TestDb::new();
    // OpenCode's shared service passed its own start-up environment to the server (measured).
    let (mut c, _) = Client::start_as(&t, claude_options(), "opencode").await;
    c.ok("register_session", json!({"client_name": "opencode"})).await;
    assert_eq!(
        count(&t.db, "conversations"),
        0,
        "no OpenCode id, and the Claude one is not ours to use"
    );
    c.close().await.unwrap();

    // An unknown client gets no link either.
    let (mut d, _) = Client::start_as(&t, claude_options(), "some-editor").await;
    d.ok("register_session", json!({"client_name": "x"})).await;
    assert_eq!(count(&t.db, "conversations"), 0);
    d.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_server_keeps_a_session_per_opencode_conversation() {
    let t = TestDb::new();
    let (mut c, _) = Client::start_as(&t, McpOptions::default(), "opencode").await;
    let (_, a) = c
        .tool_meta(
            "register_session",
            json!({"client_name": "opencode"}),
            opencode("ses_aaaaaaaa1"),
        )
        .await;
    let (_, b) = c
        .tool_meta(
            "register_session",
            json!({"client_name": "opencode"}),
            opencode("ses_bbbbbbbb2"),
        )
        .await;
    let (agent_a, agent_b) = (
        a["session"]["agent_name"].as_str().unwrap().to_owned(),
        b["session"]["agent_name"].as_str().unwrap().to_owned(),
    );
    assert_ne!(agent_a, agent_b, "two conversations are two agents");
    assert_eq!(links(&t, &agent_a)[0].conversation_id, "ses_aaaaaaaa1");
    assert_eq!(
        links(&t, &agent_b)[0].directory,
        None,
        "the server's directory is not the session's"
    );

    // Each conversation's calls are filled from its own slot.
    let (err, hb_a) = c
        .tool_meta("heartbeat", json!({}), opencode("ses_aaaaaaaa1"))
        .await;
    assert!(!err, "{hb_a}");
    assert_eq!(hb_a["id"], a["session"]["id"]);
    let (err, hb_b) = c
        .tool_meta("heartbeat", json!({}), opencode("ses_bbbbbbbb2"))
        .await;
    assert!(!err, "{hb_b}");
    assert_eq!(hb_b["id"], b["session"]["id"]);
    let (_, task) = c
        .tool_meta(
            "create_task",
            json!({"title": "from b"}),
            opencode("ses_bbbbbbbb2"),
        )
        .await;
    assert_eq!(task["creator"], json!(agent_b));

    // A conversation that has not registered borrows nobody's session; nor does a call with no id.
    let (err, v) = c
        .tool_meta("heartbeat", json!({}), opencode("ses_cccccccc3"))
        .await;
    assert!(err && v["error"]["code"] == "validation", "{v}");
    let (err, v) = c.tool("heartbeat", json!({})).await;
    assert!(err && v["error"]["code"] == "validation", "{v}");

    // EOF ends every remembered session.
    c.close().await.unwrap();
    for s in [&a, &b] {
        let id = s["session"]["id"].as_str().unwrap().to_owned();
        let ended = t.services().sessions().heartbeat(&id);
        assert!(matches!(ended, Err(CorvusError::SessionNotActive)), "{ended:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unusable_conversation_id_registers_without_a_link() {
    let t = TestDb::new();
    let (mut c, _) = Client::start_as(&t, McpOptions::default(), "opencode").await;
    for bad in ["../../etc", "has space", ""] {
        let (err, out) = c
            .tool_meta(
                "register_session",
                json!({"client_name": "opencode"}),
                opencode(bad),
            )
            .await;
        assert!(!err, "{bad:?}: {out}");
    }
    assert_eq!(count(&t.db, "conversations"), 0);
    let _ = c.close().await;
}
