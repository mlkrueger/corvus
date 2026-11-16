//! The inner router: every `/api/*` route, one handler each, one service call per handler. No auth here —
//! [`crate::app`] wraps it. The route table and the router are generated from the same list, so they cannot
//! drift apart.

use axum::extract::State;
use axum::{Json, Router};
use corvus_core::Corvus;
use corvus_core::api::{
    AckMessage, AckResult, Bootstrap, CancelResult, CancelTask, ClaimResult, ClaimTask, CompleteTask,
    ConnectResult, CreateTask, CreateThread, DecisionView, DigestView, EndStaleResult, EventView,
    ExpireResult, ExportRequest, ExportResult, Health, ListDecisions, ListEvents, ListMessages, ListSessions,
    ListTasks, MessageListItem, MessageView, Overview, PostMessage, ReadInbox, ReassignMail, ReassignResult,
    RecordDecision, RegisterSession, SendDirect, SessionRef, SessionView, Stats, TaskView, ThreadView,
    WaitForInbox, WriteDigest,
};
use corvus_core::api::{QueueTicket, SourceSummary, SourceView, SyncResult};
use corvus_domain::{AgentName, CorvusError, SessionId, TaskId, ThreadId};
use corvus_domain::{SourceFilter, SourceId};
use serde::Deserialize;

use crate::error::ApiError;
use crate::extract::{Body, OptBody, P, Q};

type Res<T> = Result<Json<T>, ApiError>;

fn out<T>(r: corvus_domain::Result<T>) -> Res<T> {
    r.map(Json).map_err(ApiError::from)
}

macro_rules! routes {
    ($( $verb:ident $method:literal $path:literal => $handler:ident ),+ $(,)?) => {
        /// Every route `data_router` serves, as `(method, path template)`. The router is built from this list.
        pub fn route_table() -> &'static [(&'static str, &'static str)] {
            &[ $( ($method, $path) ),+ ]
        }

        /// The inner router: all `/api/*` handlers, **no authentication**. Never bind it to a socket; wrap it
        /// with [`crate::app`] (host allow-list, bearer auth, limits) or call it in memory.
        pub fn data_router(corvus: Corvus) -> Router {
            Router::new()
                $( .route($path, axum::routing::$verb($handler)) )+
                .with_state(corvus)
        }
    };
}

routes! {
    get "GET" "/api/health" => health,
    get "GET" "/api/stats" => stats,
    get "GET" "/api/overview" => overview,
    post "POST" "/api/sessions" => register_session,
    get "GET" "/api/sessions" => list_sessions,
    post "POST" "/api/connect" => connect,
    post "POST" "/api/sessions/{id}/heartbeat" => heartbeat,
    post "POST" "/api/sessions/{id}/end" => end_session,
    post "POST" "/api/tasks" => create_task,
    get "GET" "/api/tasks" => list_tasks,
    get "GET" "/api/tasks/{id}" => get_task,
    post "POST" "/api/tasks/claim" => claim_task,
    post "POST" "/api/tasks/{id}/renew" => renew_lease,
    post "POST" "/api/tasks/{id}/release" => release_task,
    post "POST" "/api/tasks/{id}/complete" => complete_task,
    post "POST" "/api/tasks/{id}/cancel" => cancel_task,
    post "POST" "/api/threads" => create_thread,
    get "GET" "/api/threads/{id}" => get_thread,
    get "GET" "/api/threads/{id}/messages" => thread_messages,
    post "POST" "/api/messages" => post_message,
    get "GET" "/api/messages" => list_messages,
    post "POST" "/api/messages/direct" => send_direct,
    post "POST" "/api/messages/{id}/ack" => ack_message,
    get "GET" "/api/inbox/{agent}" => read_inbox,
    get "GET" "/api/inbox/{agent}/wait" => wait_for_inbox,
    post "POST" "/api/digests" => write_digest,
    get "GET" "/api/digests" => list_digests,
    get "GET" "/api/digests/latest" => latest_digest,
    get "GET" "/api/bootstrap/{agent}" => bootstrap,
    post "POST" "/api/decisions" => record_decision,
    get "GET" "/api/decisions" => list_decisions,
    get "GET" "/api/events" => list_events,
    post "POST" "/api/admin/expire-leases" => expire_leases,
    post "POST" "/api/admin/end-stale-sessions" => end_stale_sessions,
    post "POST" "/api/admin/export" => export_state,
    post "POST" "/api/admin/reassign-mail" => reassign_mail,
    get "GET" "/api/sources" => list_sources,
    get "GET" "/api/sources/{source}" => get_source,
    put "PUT" "/api/sources/{source}/filter" => put_source_filter,
    post "POST" "/api/sources/{source}/sync" => sync_source,
    post "POST" "/api/sources/{source}/queue" => queue_ticket,
}

// ---------- path-parameter validation (422 before any service call) ----------

fn session_id(raw: &str) -> Result<String, ApiError> {
    Ok(SessionId::parse(raw)?.to_string())
}

fn task_id(raw: &str) -> Result<String, ApiError> {
    Ok(TaskId::parse(raw)?.to_string())
}

fn thread_id(raw: &str) -> Result<String, ApiError> {
    Ok(ThreadId::parse(raw)?.to_string())
}

fn agent_name(raw: &str) -> Result<String, ApiError> {
    Ok(AgentName::parse(raw)?.to_string())
}

fn message_id(raw: &str) -> Result<i64, ApiError> {
    raw.parse::<i64>()
        .map_err(|_| CorvusError::Validation("invalid message id".into()).into())
}

// ---------- query shapes that differ from the api structs (the path carries a field, or names differ) ----------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    after_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitQuery {
    /// Seconds; the service clamps to 0..=120.
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LatestQuery {
    scope: String,
    #[serde(rename = "ref", default)]
    reference: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsQuery {
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    thread: Option<String>,
    #[serde(default)]
    include_superseded: bool,
    #[serde(default)]
    limit: Option<u32>,
}

// ---------- handlers ----------

async fn health(State(c): State<Corvus>) -> Res<Health> {
    out(c.run(|s| s.overview().health()).await)
}

async fn stats(State(c): State<Corvus>) -> Res<Stats> {
    out(c.run(|s| s.overview().stats()).await)
}

async fn overview(State(c): State<Corvus>) -> Res<Overview> {
    out(c.run(|s| s.overview().overview()).await)
}

async fn register_session(State(c): State<Corvus>, Body(req): Body<RegisterSession>) -> Res<SessionView> {
    out(c.run(move |s| s.sessions().register(&req)).await)
}

async fn list_sessions(State(c): State<Corvus>, Q(req): Q<ListSessions>) -> Res<Vec<SessionView>> {
    out(c.run(move |s| s.sessions().list(&req)).await)
}

async fn connect(State(c): State<Corvus>, Body(req): Body<RegisterSession>) -> Res<ConnectResult> {
    out(c.run(move |s| s.muninn().connect(&req)).await)
}

async fn heartbeat(State(c): State<Corvus>, P(id): P) -> Res<SessionView> {
    let id = session_id(&id)?;
    out(c.run(move |s| s.sessions().heartbeat(&id)).await)
}

async fn end_session(State(c): State<Corvus>, P(id): P) -> Res<SessionView> {
    let id = session_id(&id)?;
    out(c.run(move |s| s.sessions().end(&id)).await)
}

async fn create_task(State(c): State<Corvus>, Body(req): Body<CreateTask>) -> Res<TaskView> {
    out(c.run(move |s| s.huginn().create_task(&req)).await)
}

async fn list_tasks(State(c): State<Corvus>, Q(req): Q<ListTasks>) -> Res<Vec<TaskView>> {
    out(c.run(move |s| s.huginn().list_tasks(&req)).await)
}

async fn get_task(State(c): State<Corvus>, P(id): P) -> Res<TaskView> {
    let id = task_id(&id)?;
    out(c.run(move |s| s.huginn().get_task(&id)).await)
}

async fn claim_task(State(c): State<Corvus>, Body(req): Body<ClaimTask>) -> Res<ClaimResult> {
    out(c.run(move |s| s.huginn().claim_task(&req)).await)
}

async fn renew_lease(State(c): State<Corvus>, P(id): P, Body(req): Body<SessionRef>) -> Res<TaskView> {
    let id = task_id(&id)?;
    out(c.run(move |s| s.huginn().renew_lease(&id, &req.session_id)).await)
}

async fn release_task(State(c): State<Corvus>, P(id): P, Body(req): Body<SessionRef>) -> Res<TaskView> {
    let id = task_id(&id)?;
    out(c
        .run(move |s| s.huginn().release_task(&id, &req.session_id))
        .await)
}

async fn complete_task(State(c): State<Corvus>, P(id): P, Body(req): Body<CompleteTask>) -> Res<TaskView> {
    let id = task_id(&id)?;
    out(c.run(move |s| s.huginn().complete_task(&id, &req)).await)
}

async fn cancel_task(State(c): State<Corvus>, P(id): P, Body(req): Body<CancelTask>) -> Res<CancelResult> {
    let id = task_id(&id)?;
    out(c.run(move |s| s.huginn().cancel_task(&id, &req)).await)
}

async fn create_thread(State(c): State<Corvus>, Body(req): Body<CreateThread>) -> Res<ThreadView> {
    out(c.run(move |s| s.muninn().create_thread(&req)).await)
}

async fn get_thread(State(c): State<Corvus>, P(id): P) -> Res<ThreadView> {
    let id = thread_id(&id)?;
    out(c.run(move |s| s.muninn().get_thread(&id)).await)
}

async fn thread_messages(State(c): State<Corvus>, P(id): P, Q(q): Q<PageQuery>) -> Res<Vec<MessageView>> {
    let id = thread_id(&id)?;
    out(c
        .run(move |s| s.muninn().thread_messages(&id, q.limit, q.after_id))
        .await)
}

async fn post_message(State(c): State<Corvus>, Body(req): Body<PostMessage>) -> Res<MessageView> {
    out(c.run(move |s| s.muninn().post_message(&req)).await)
}

async fn list_messages(State(c): State<Corvus>, Q(req): Q<ListMessages>) -> Res<Vec<MessageListItem>> {
    out(c.run(move |s| s.muninn().list_messages(&req)).await)
}

async fn send_direct(State(c): State<Corvus>, Body(req): Body<SendDirect>) -> Res<MessageView> {
    out(c.run(move |s| s.muninn().send_direct(&req)).await)
}

async fn ack_message(State(c): State<Corvus>, P(id): P, Body(req): Body<AckMessage>) -> Res<AckResult> {
    let id = message_id(&id)?;
    out(c.run(move |s| s.muninn().ack_message(id, &req)).await)
}

async fn read_inbox(State(c): State<Corvus>, P(agent): P, Q(q): Q<PageQuery>) -> Res<Vec<MessageView>> {
    let req = ReadInbox {
        agent: agent_name(&agent)?,
        limit: q.limit,
        after_id: q.after_id,
    };
    out(c.run(move |s| s.muninn().read_inbox(&req)).await)
}

async fn wait_for_inbox(State(c): State<Corvus>, P(agent): P, Q(q): Q<WaitQuery>) -> Res<Vec<MessageView>> {
    let req = WaitForInbox {
        agent: agent_name(&agent)?,
        timeout_seconds: q.timeout,
        session_id: q.session_id,
        limit: q.limit,
    };
    out(c.wait_for_inbox(req).await)
}

async fn write_digest(State(c): State<Corvus>, Body(req): Body<WriteDigest>) -> Res<DigestView> {
    out(c.run(move |s| s.muninn().write_digest(&req)).await)
}

async fn list_digests(State(c): State<Corvus>) -> Res<Vec<DigestView>> {
    out(c.run(|s| s.muninn().list_latest_digests()).await)
}

/// `null` when there is no digest for the scope.
async fn latest_digest(State(c): State<Corvus>, Q(q): Q<LatestQuery>) -> Res<Option<DigestView>> {
    out(c
        .run(move |s| s.muninn().latest_digest(&q.scope, q.reference.as_deref()))
        .await)
}

async fn bootstrap(State(c): State<Corvus>, P(agent): P) -> Res<Bootstrap> {
    let agent = agent_name(&agent)?;
    out(c.run(move |s| s.muninn().bootstrap(&agent)).await)
}

async fn record_decision(State(c): State<Corvus>, Body(req): Body<RecordDecision>) -> Res<DecisionView> {
    out(c.run(move |s| s.muninn().record_decision(&req)).await)
}

async fn list_decisions(State(c): State<Corvus>, Q(q): Q<DecisionsQuery>) -> Res<Vec<DecisionView>> {
    let req = ListDecisions {
        task_id: q.task,
        thread_id: q.thread,
        include_superseded: q.include_superseded,
        limit: q.limit,
    };
    out(c.run(move |s| s.muninn().list_decisions(&req)).await)
}

async fn list_events(State(c): State<Corvus>, Q(req): Q<ListEvents>) -> Res<Vec<EventView>> {
    out(c.run(move |s| s.overview().list_events(&req)).await)
}

async fn expire_leases(State(c): State<Corvus>) -> Res<ExpireResult> {
    out(c.run(|s| s.huginn().expire_leases()).await)
}

async fn end_stale_sessions(State(c): State<Corvus>) -> Res<EndStaleResult> {
    out(c.run(|s| s.sessions().end_stale()).await)
}

async fn export_state(State(c): State<Corvus>, OptBody(req): OptBody<ExportRequest>) -> Res<ExportResult> {
    out(c.run(move |s| s.export().export_state_to_git(&req)).await)
}

/// Operator action (D0025). Deliberately not an MCP tool: agents cannot move each other's mail.
async fn reassign_mail(State(c): State<Corvus>, Body(req): Body<ReassignMail>) -> Res<ReassignResult> {
    out(c.run(move |s| s.muninn().reassign_mail(&req)).await)
}

// ---------- work sources ----------

fn source(p: &str) -> Result<SourceId, ApiError> {
    SourceId::parse(p).map_err(ApiError::from)
}

async fn list_sources(State(c): State<Corvus>) -> Res<Vec<SourceSummary>> {
    Ok(Json(c.sources().list()))
}

async fn get_source(State(c): State<Corvus>, P(id): P) -> Res<SourceView> {
    let id = source(&id)?;
    out(c.sources().view(id).await)
}

async fn put_source_filter(
    State(c): State<Corvus>,
    P(id): P,
    Body(f): Body<SourceFilter>,
) -> Res<SourceView> {
    let id = source(&id)?;
    out(c.sources().save_filter(id, f).await)
}

async fn sync_source(State(c): State<Corvus>, P(id): P) -> Res<SyncResult> {
    let id = source(&id)?;
    out(c.sources().sync(id).await)
}

async fn queue_ticket(State(c): State<Corvus>, P(id): P, Body(req): Body<QueueTicket>) -> Res<TaskView> {
    let id = source(&id)?;
    out(c.sources().queue(id, &req).await)
}
