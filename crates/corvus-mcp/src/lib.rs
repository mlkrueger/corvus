//! The MCP server (`corvus mcp`, stdio) ([system/04](../../../spec/system/04-interfaces.md#mcp-server-corvus-mcp-stdio)).
//!
//! Twenty tools, each one service call. Input schemas are derived from the request structs in
//! `corvus_core::api` (the HTTP handlers deserialise the very same structs); the only schema edits are the
//! ones the session convenience needs: fields the process can fill in (`session_id`, agent names) stop being
//! required, and the id that is an HTTP path parameter (`task_id`, `message_id`) becomes a property.
//!
//! stdout carries protocol only; diagnostics go through `tracing` (stderr).

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use corvus_core::Corvus;
use corvus_core::api::{
    AckMessage, CancelTask, ClaimTask, CompleteTask, ConversationInput, CreateTask, CreateThread,
    ExportRequest, ListDecisions, PostMessage, ReadInbox, RecordDecision, RegisterSession, SendDirect,
    SessionRef, WaitForInbox, WriteDigest,
};
use corvus_core::services::Services;
use corvus_domain::{AgentName, ConversationRef, CorvusError, GUIDANCE, Tool as AgentTool};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

/// What the process that started this server knows about its host.
#[derive(Debug, Clone, Default)]
pub struct McpOptions {
    /// `CORVUS_AGENT_NAME`, used when `register_session` omits a name.
    pub default_agent: Option<AgentName>,
    /// `CLAUDE_CODE_SESSION_ID` from the environment. Used as the conversation id **only** when the client
    /// says it is Claude Code: other tools can leak the variable into the servers they start.
    pub claude_session_id: Option<String>,
    /// The working directory, recorded with a Claude Code conversation link.
    pub directory: Option<String>,
}

/// The MCP `clientInfo.name` Claude Code sends (measured on 2.1.325, in each request's `_meta`).
const CLIENT_CLAUDE_CODE: &str = "claude-code";
/// The `clientInfo.name` OpenCode sends at `initialize` (measured on 2.3.0).
const CLIENT_OPENCODE: &str = "opencode";
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
const META_OPENCODE_SESSION: &str = "ai.opencode/sessionID";

/// Serves MCP over stdin/stdout until stdin closes; then ends the remembered sessions (best effort).
pub async fn run_stdio(corvus: Corvus, options: McpOptions) -> Result<(), CorvusError> {
    let (reader, writer) = rmcp::transport::stdio();
    run(corvus, options, reader, writer).await
}

/// [`run_stdio`] over any reader/writer pair (tests use an in-memory pipe).
pub async fn run<R, W>(corvus: Corvus, options: McpOptions, reader: R, writer: W) -> Result<(), CorvusError>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let server = CorvusMcp::new(corvus, options);
    let outcome = match server.clone().serve((reader, writer)).await {
        Ok(running) => running.waiting().await.map(|_| ()).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    server.end_remembered_sessions().await;
    outcome.map_err(|e| {
        tracing::error!("mcp transport failed: {e}");
        CorvusError::Internal(format!("mcp transport failed: {e}"))
    })
}

#[derive(Debug, Clone)]
struct Remembered {
    session_id: String,
    agent: String,
}

/// The memory slot a call belongs to: its conversation, or the one shared slot for calls without one.
type Slot = Option<ConversationRef>;

#[derive(Clone)]
struct CorvusMcp {
    corvus: Corvus,
    options: McpOptions,
    /// One process can serve several conversations (OpenCode runs one server for all its sessions), so the
    /// remembered session is kept per conversation.
    remembered: Arc<Mutex<HashMap<Slot, Remembered>>>,
    tools: Arc<Vec<ToolDef>>,
}

/// How a tool's arguments are completed before they become the core request.
struct ToolDef {
    name: &'static str,
    description: &'static str,
    /// Fields holding an agent name: omitted or empty means the remembered agent.
    agent_fields: &'static [&'static str],
    /// The request has a `session_id`: omitted or empty means the remembered session.
    session: bool,
    /// The HTTP path parameter that is an extra argument here.
    id: Option<IdField>,
    schema: Arc<JsonObject>,
}

#[derive(Clone, Copy)]
struct IdField {
    name: &'static str,
    integer: bool,
}

const TASK_ID: IdField = IdField {
    name: "task_id",
    integer: false,
};
const MESSAGE_ID: IdField = IdField {
    name: "message_id",
    integer: true,
};

/// `get_bootstrap_context` has no core request struct (the service takes a bare agent name).
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
#[allow(dead_code)]
struct BootstrapContext {
    /// Defaults to the agent registered by this process.
    agent: Option<String>,
}

/// Builds the schema from the core request type `T`, then relaxes fields the process fills in and adds the id.
fn tool<T: JsonSchema>(
    name: &'static str,
    description: &'static str,
    agent_fields: &'static [&'static str],
    session: bool,
    id: Option<IdField>,
) -> ToolDef {
    let mut schema = match serde_json::to_value(schemars::schema_for!(T)) {
        Ok(Value::Object(map)) => map,
        _ => JsonObject::new(),
    };
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        required.retain(|r| {
            let r = r.as_str().unwrap_or_default();
            !(agent_fields.contains(&r) || (session && r == "session_id"))
        });
    }
    if schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        schema.remove("required");
    }
    if let Some(id) = id {
        let ty = if id.integer { "integer" } else { "string" };
        if let Some(Value::Object(props)) = schema.get_mut("properties") {
            props.insert(id.name.into(), json!({ "type": ty }));
        } else {
            schema.insert("properties".into(), json!({ id.name: { "type": ty } }));
        }
        match schema.get_mut("required") {
            Some(Value::Array(required)) => required.push(id.name.into()),
            _ => {
                schema.insert("required".into(), json!([id.name]));
            }
        }
    }
    ToolDef {
        name,
        description,
        agent_fields,
        session,
        id,
        schema: Arc::new(schema),
    }
}

fn tool_defs() -> Vec<ToolDef> {
    vec![
        tool::<RegisterSession>(
            "register_session",
            "Register this agent session. Returns the session, bootstrap context and guidance. The session id \
             and agent name are remembered, so later calls may omit them.",
            &[],
            false,
            None,
        ),
        tool::<BootstrapContext>(
            "get_bootstrap_context",
            "Everything an agent needs on arrival: routed and claimable tasks, unread messages, digests, decisions.",
            &["agent"],
            false,
            None,
        ),
        tool::<CreateTask>("create_task", "Create a task.", &["creator"], true, None),
        tool::<ClaimTask>(
            "claim_task",
            "Claim the next claimable task (highest priority first). Both fields of the result are null when \
             nothing is claimable.",
            &[],
            true,
            None,
        ),
        tool::<SessionRef>(
            "renew_lease",
            "Extend the lease on a task you hold.",
            &[],
            true,
            Some(TASK_ID),
        ),
        tool::<SessionRef>(
            "release_task",
            "Give a claimed task back so another agent can claim it.",
            &[],
            true,
            Some(TASK_ID),
        ),
        tool::<CompleteTask>(
            "complete_task",
            "Mark a task you hold as done.",
            &[],
            true,
            Some(TASK_ID),
        ),
        tool::<CancelTask>(
            "cancel_task",
            "Cancel a task, optionally with every dependent.",
            &["by"],
            true,
            Some(TASK_ID),
        ),
        tool::<PostMessage>(
            "post_message",
            "Post a message to a thread, to one agent or broadcast.",
            &["from"],
            true,
            None,
        ),
        tool::<CreateThread>(
            "create_thread",
            "Create a thread (optionally linked to a task).",
            &["creator"],
            true,
            None,
        ),
        tool::<SendDirect>(
            "send_direct_message",
            "Send a direct message to one agent.",
            &["from"],
            true,
            None,
        ),
        tool::<ReadInbox>(
            "read_inbox",
            "Read your unread messages. Reading does not acknowledge them; use ack_message.",
            &["agent"],
            false,
            None,
        ),
        tool::<WaitForInbox>(
            "wait_for_inbox",
            "Block until you have unread messages or the timeout (at most 120 seconds) passes.",
            &["agent"],
            true,
            None,
        ),
        tool::<AckMessage>(
            "ack_message",
            "Acknowledge a message as read.",
            &["agent"],
            true,
            Some(MESSAGE_ID),
        ),
        tool::<SessionRef>("heartbeat", "Tell Corvus this session is alive.", &[], true, None),
        tool::<SessionRef>(
            "end_session",
            "End this session; its claims are released.",
            &[],
            true,
            None,
        ),
        tool::<WriteDigest>(
            "write_digest",
            "Write a summary digest for a scope (global, task, thread or agent).",
            &["author"],
            true,
            None,
        ),
        tool::<RecordDecision>(
            "record_decision",
            "Record a decision, optionally superseding an earlier one.",
            &["decided_by"],
            true,
            None,
        ),
        tool::<ListDecisions>("list_decisions", "List recorded decisions.", &[], false, None),
        tool::<ExportRequest>(
            "export_state_to_git",
            "Export coordination state as files into the configured export repository.",
            &[],
            false,
            None,
        ),
    ]
}

fn error_json(err: &CorvusError) -> String {
    let message = match err {
        CorvusError::Internal(detail) => {
            tracing::error!("internal error: {detail}");
            "internal error".to_owned()
        }
        other => other.to_string(),
    };
    json!({ "error": { "code": err.code(), "message": message } }).to_string()
}

fn blank(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

fn parse<T: DeserializeOwned>(map: JsonObject) -> Result<T, CorvusError> {
    serde_json::from_value(Value::Object(map)).map_err(|e| CorvusError::Validation(e.to_string()))
}

impl CorvusMcp {
    fn new(corvus: Corvus, options: McpOptions) -> Self {
        Self {
            corvus,
            options,
            remembered: Arc::new(Mutex::new(HashMap::new())),
            tools: Arc::new(tool_defs()),
        }
    }

    fn memory(&self, slot: &Slot) -> Option<Remembered> {
        self.remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(slot)
            .cloned()
    }

    async fn end_remembered_sessions(&self) {
        let all: Vec<Remembered> = self
            .remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for m in all {
            let id = m.session_id;
            if let Err(e) = self.corvus.run(move |s| s.sessions().end(&id)).await {
                tracing::debug!("ending remembered session on exit: {e}");
            }
        }
    }

    /// The tool conversation a call belongs to, taken only from the source that belongs to the client that
    /// made it. An id that fails validation is dropped: the call proceeds with no conversation.
    fn conversation(&self, client: Option<&str>, meta: &JsonObject) -> Slot {
        let (tool, id) = match client {
            Some(CLIENT_CLAUDE_CODE) => (AgentTool::ClaudeCode, self.options.claude_session_id.as_deref()?),
            Some(CLIENT_OPENCODE) => (AgentTool::OpenCode, meta.get(META_OPENCODE_SESSION)?.as_str()?),
            _ => return None,
        };
        ConversationRef::new(tool, id).ok()
    }

    /// Fills omitted fields from the remembered session and splits off the path-style id.
    fn prepare(
        &self,
        def: &ToolDef,
        args: Option<JsonObject>,
        slot: &Slot,
    ) -> Result<(JsonObject, Option<Value>), CorvusError> {
        let mut map = args.unwrap_or_default();
        let id = match def.id {
            Some(f) => {
                let v = map.remove(f.name);
                let ok = match &v {
                    Some(Value::String(s)) => !f.integer && !s.is_empty(),
                    Some(Value::Number(n)) => f.integer && n.is_i64(),
                    _ => false,
                };
                if !ok {
                    let want = if f.integer { "an integer" } else { "a string" };
                    return Err(CorvusError::Validation(format!(
                        "`{}` is required and must be {want}",
                        f.name
                    )));
                }
                v
            }
            None => None,
        };
        let memory = self.memory(slot);
        if let Some(m) = &memory {
            for f in def.agent_fields {
                if blank(map.get(*f)) {
                    map.insert((*f).to_owned(), Value::String(m.agent.clone()));
                }
            }
            // The remembered session belongs to the remembered agent: do not attach it to a call made
            // explicitly on behalf of someone else.
            let acting_as_self = def
                .agent_fields
                .iter()
                .all(|f| map.get(*f).and_then(Value::as_str) == Some(&m.agent));
            if def.session && acting_as_self && blank(map.get("session_id")) {
                map.insert("session_id".to_owned(), Value::String(m.session_id.clone()));
            }
        }
        if def.session && map.get("session_id").is_some_and(Value::is_null) {
            map.remove("session_id");
        }
        Ok((map, id))
    }

    async fn svc<T, F>(&self, f: F) -> Result<Value, CorvusError>
    where
        T: Serialize + Send + 'static,
        F: FnOnce(&Services) -> Result<T, CorvusError> + Send + 'static,
    {
        let out = self.corvus.run(f).await?;
        serde_json::to_value(out).map_err(|e| CorvusError::Internal(format!("serialise result: {e}")))
    }

    async fn dispatch(&self, name: &str, args: Option<JsonObject>, slot: Slot) -> Result<Value, CorvusError> {
        let def = self
            .tools
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| CorvusError::NotFound(format!("unknown tool `{name}`")))?;
        let (mut map, id) = self.prepare(def, args, &slot)?;
        let task = || id.as_ref().and_then(Value::as_str).unwrap_or_default().to_owned();
        match name {
            "register_session" => {
                if blank(map.get("agent_name")) {
                    map.remove("agent_name");
                    if let Some(a) = &self.options.default_agent {
                        map.insert("agent_name".into(), Value::String(a.to_string()));
                    }
                }
                // The server says which conversation this is; a model cannot.
                map.remove("conversation");
                let mut req: RegisterSession = parse(map)?;
                req.conversation = slot.as_ref().map(|c| ConversationInput {
                    tool: c.tool().to_string(),
                    id: c.id().to_owned(),
                    directory: (c.tool() == AgentTool::ClaudeCode)
                        .then(|| self.options.directory.clone())
                        .flatten(),
                });
                let out = self.corvus.run(move |s| s.muninn().connect(&req)).await?;
                self.remembered
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(
                        slot,
                        Remembered {
                            session_id: out.session.id.clone(),
                            agent: out.session.agent_name.clone(),
                        },
                    );
                serde_json::to_value(out).map_err(|e| CorvusError::Internal(format!("serialise result: {e}")))
            }
            "get_bootstrap_context" => {
                let agent = map
                    .get("agent")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| {
                        CorvusError::Validation("`agent` is required (register_session first)".into())
                    })?
                    .to_owned();
                self.svc(move |s| s.muninn().bootstrap(&agent)).await
            }
            "create_task" => {
                let req: CreateTask = parse(map)?;
                self.svc(move |s| s.huginn().create_task(&req)).await
            }
            "claim_task" => {
                let req: ClaimTask = parse(map)?;
                self.svc(move |s| s.huginn().claim_task(&req)).await
            }
            "renew_lease" | "release_task" => {
                let req: SessionRef = parse(map)?;
                let (task, renew) = (task(), name == "renew_lease");
                self.svc(move |s| {
                    if renew {
                        s.huginn().renew_lease(&task, &req.session_id)
                    } else {
                        s.huginn().release_task(&task, &req.session_id)
                    }
                })
                .await
            }
            "complete_task" => {
                let req: CompleteTask = parse(map)?;
                let task = task();
                self.svc(move |s| s.huginn().complete_task(&task, &req)).await
            }
            "cancel_task" => {
                let req: CancelTask = parse(map)?;
                let task = task();
                self.svc(move |s| s.huginn().cancel_task(&task, &req)).await
            }
            "post_message" => {
                let req: PostMessage = parse(map)?;
                self.svc(move |s| s.muninn().post_message(&req)).await
            }
            "create_thread" => {
                let req: CreateThread = parse(map)?;
                self.svc(move |s| s.muninn().create_thread(&req)).await
            }
            "send_direct_message" => {
                let req: SendDirect = parse(map)?;
                self.svc(move |s| s.muninn().send_direct(&req)).await
            }
            "read_inbox" => {
                let req: ReadInbox = parse(map)?;
                self.svc(move |s| s.muninn().read_inbox(&req)).await
            }
            "wait_for_inbox" => {
                let req: WaitForInbox = parse(map)?;
                let out = self.corvus.wait_for_inbox(req).await?;
                serde_json::to_value(out).map_err(|e| CorvusError::Internal(format!("serialise result: {e}")))
            }
            "ack_message" => {
                let req: AckMessage = parse(map)?;
                let message_id = id.as_ref().and_then(Value::as_i64).unwrap_or_default();
                self.svc(move |s| s.muninn().ack_message(message_id, &req)).await
            }
            "heartbeat" | "end_session" => {
                let req: SessionRef = parse(map)?;
                let end = name == "end_session";
                self.svc(move |s| {
                    if end {
                        s.sessions().end(&req.session_id)
                    } else {
                        s.sessions().heartbeat(&req.session_id)
                    }
                })
                .await
            }
            "write_digest" => {
                let req: WriteDigest = parse(map)?;
                self.svc(move |s| s.muninn().write_digest(&req)).await
            }
            "record_decision" => {
                let req: RecordDecision = parse(map)?;
                self.svc(move |s| s.muninn().record_decision(&req)).await
            }
            "list_decisions" => {
                let req: ListDecisions = parse(map)?;
                self.svc(move |s| s.muninn().list_decisions(&req)).await
            }
            "export_state_to_git" => {
                let req: ExportRequest = parse(map)?;
                self.svc(move |s| s.export().export_state_to_git(&req)).await
            }
            other => Err(CorvusError::NotFound(format!("unknown tool `{other}`"))),
        }
    }
}

impl ServerHandler for CorvusMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(GUIDANCE)
            .with_server_info(Implementation::new("corvus", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let tools: Vec<Tool> = self
            .tools
            .iter()
            .map(|d| {
                Tool::new(
                    Cow::Borrowed(d.name),
                    Cow::Borrowed(d.description),
                    d.schema.clone(),
                )
            })
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Claude Code names itself in each request's `_meta`; OpenCode at `initialize`.
        let meta: &JsonObject = request.meta.as_deref().map_or(&context.meta, |m| m);
        let peer = context.peer.peer_info();
        let client = meta
            .get(META_CLIENT_INFO)
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
            .or_else(|| peer.as_deref().map(|p| p.client_info.name.as_str()));
        let slot = self.conversation(client, meta);
        let result = match self.dispatch(&request.name, request.arguments, slot).await {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(error_json(&e))]),
        };
        Ok(CallToolResponse::from(result))
    }
}
