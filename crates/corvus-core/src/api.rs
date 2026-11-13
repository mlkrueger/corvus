//! Request and response shapes shared by every transport (system/04). One definition of each shape: the HTTP
//! handlers deserialise these, the MCP tools derive their schemas from them, and the CLI builds them.
//!
//! Requests carry plain strings; services validate them into domain types and return `Validation` errors.
//! Responses carry RFC 3339 timestamps.

use corvus_domain::{ArtifactKind, EndReason, MessageType, Presence, SessionEndReason, TaskStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::Timestamp;

// ---------- sessions ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct RegisterSession {
    /// Omit to have Corvus generate and reserve a name like `keen-raven`.
    #[serde(default)]
    pub agent_name: Option<String>,
    pub client_name: String,
    /// Free-form JSON object.
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionView {
    pub id: String,
    pub agent_name: String,
    pub client_name: String,
    pub metadata: Value,
    pub name_generated: bool,
    pub started_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub end_reason: Option<SessionEndReason>,
    pub presence: Presence,
    /// The task this session holds an active claim on, if any.
    pub current_task: Option<String>,
    pub lease_expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ListSessions {
    /// Include ended sessions (default false).
    #[serde(default)]
    pub include_ended: bool,
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndStaleResult {
    /// Ids of the sessions this pass ended.
    pub ended: Vec<String>,
}

// ---------- tasks ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInput {
    pub kind: ArtifactKind,
    pub path: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ArtifactView {
    pub id: String,
    pub kind: ArtifactKind,
    pub path: String,
    pub commit: Option<String>,
    pub description: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    pub creator: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// Higher is served first. Default 0.
    #[serde(default)]
    pub priority: Option<i64>,
    /// Routing bucket. Default `default`.
    #[serde(default)]
    pub pool: Option<String>,
    /// Only this agent may claim it.
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactInput>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Optional; when given it must be an active session of `creator` and counts as a heartbeat.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Holder {
    pub agent_name: String,
    pub session_id: String,
    pub claim_id: String,
    pub claimed_at: Timestamp,
    pub lease_expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskView {
    pub id: String,
    pub title: String,
    pub body: String,
    pub priority: i64,
    pub status: TaskStatus,
    pub pool: String,
    pub recipient_agent: Option<String>,
    pub creator: String,
    pub result_summary: Option<String>,
    pub cancel_reason: Option<String>,
    pub cancelled_by: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub cancelled_at: Option<Timestamp>,
    pub thread_id: Option<String>,
    pub depends_on: Vec<String>,
    pub artifacts: Vec<ArtifactView>,
    /// Some dependency is not `done`.
    pub blocked: bool,
    /// Some dependency is `cancelled` (so this task will stay blocked).
    pub blocked_by_cancelled: bool,
    pub holder: Option<Holder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClaimView {
    pub id: String,
    pub task_id: String,
    pub session_id: String,
    pub agent_name: String,
    pub claimed_at: Timestamp,
    pub lease_expires_at: Timestamp,
    pub released_at: Option<Timestamp>,
    pub end_reason: Option<EndReason>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ClaimTask {
    pub session_id: String,
    #[serde(default)]
    pub pool: Option<String>,
}

/// `claim_task`'s result: both `null` when nothing is claimable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClaimResult {
    pub task: Option<TaskView>,
    pub claim: Option<ClaimView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SessionRef {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CompleteTask {
    pub session_id: String,
    #[serde(default)]
    pub result_summary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CancelTask {
    /// The agent cancelling (the operator, the creator, or anyone).
    pub by: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// Also cancel every transitive dependent (done ones are skipped).
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CancelResult {
    pub task: TaskView,
    /// Ids cancelled by this call (empty on an idempotent replay).
    pub cancelled: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ListTasks {
    #[serde(default)]
    pub status: Option<TaskStatus>,
    /// Tasks this agent created, is routed, or holds.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExpireResult {
    pub expired: usize,
}

// ---------- threads and messages (plan 0003) ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CreateThread {
    pub creator: String,
    pub title: String,
    /// Link to a task (a task has one thread; asking again returns it).
    #[serde(default)]
    pub task_id: Option<String>,
    /// Same key, same thread. Keys starting with `direct:` are reserved.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ThreadView {
    pub id: String,
    pub title: String,
    pub task_id: Option<String>,
    pub status: String,
    pub creator: String,
    pub key: Option<String>,
    pub created_at: Timestamp,
    pub artifacts: Vec<ArtifactView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PostMessage {
    pub thread_id: String,
    pub from: String,
    pub body: String,
    #[serde(rename = "type", default = "default_message_type")]
    pub kind: MessageType,
    /// Recipient agent; omit to broadcast to everyone.
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<i64>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactInput>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_message_type() -> MessageType {
    MessageType::Note
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageView {
    pub id: i64,
    pub thread_id: String,
    pub sender: String,
    pub recipient: Option<String>,
    #[serde(rename = "type")]
    pub kind: MessageType,
    pub in_reply_to: Option<i64>,
    pub body: String,
    pub session_id: Option<String>,
    pub created_at: Timestamp,
    pub artifacts: Vec<ArtifactView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SendDirect {
    pub from: String,
    pub to: String,
    pub body: String,
    #[serde(rename = "type", default = "default_message_type")]
    pub kind: MessageType,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ReadInbox {
    pub agent: String,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Paging hint only; unread is a query, not a cursor.
    #[serde(default)]
    pub after_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct AckMessage {
    pub agent: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AckResult {
    pub message_id: i64,
    pub newly_acked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ListMessages {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub before_id: Option<i64>,
    /// Sender or recipient.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub thread: Option<String>,
    /// Only messages someone in their audience has not read.
    #[serde(default)]
    pub unread_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageListItem {
    #[serde(flatten)]
    pub message: MessageView,
    pub thread_title: String,
    pub read_by: Vec<String>,
    pub unread_by: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct WaitForInbox {
    pub agent: String,
    /// Seconds, clamped to 0..=120. Default 60.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

// ---------- memory ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct WriteDigest {
    pub author: String,
    /// `global | task | thread | agent`.
    pub scope: String,
    /// Task id, thread id or agent name; omit for `global`.
    #[serde(rename = "ref", default)]
    pub reference: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub covers_through: Option<i64>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DigestView {
    pub id: i64,
    pub scope: String,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub scope_key: String,
    pub summary: String,
    pub author: String,
    pub covers_through: Option<i64>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct RecordDecision {
    pub decided_by: String,
    pub title: String,
    pub decision: String,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionView {
    pub id: String,
    pub title: String,
    pub decision: String,
    pub rationale: String,
    pub decided_by: String,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ListDecisions {
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub include_superseded: bool,
    #[serde(default)]
    pub limit: Option<u32>,
}

// ---------- bootstrap, connect, stats, overview, health ----------

/// Everything an agent needs on arrival (system/03 "Agent guidance").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Bootstrap {
    pub agent: String,
    /// Routed to this agent and open, or claimed by it.
    pub routed_tasks: Vec<TaskView>,
    /// Open tasks this agent could claim now.
    pub claimable_tasks: Vec<TaskView>,
    pub unread_messages: Vec<MessageView>,
    /// Questions addressed to this agent (or broadcast) with no answer yet.
    pub unresolved_questions: Vec<MessageView>,
    pub digests: Vec<DigestView>,
    pub decisions: Vec<DecisionView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConnectResult {
    pub session: SessionView,
    pub bootstrap: Bootstrap,
    pub guidance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Stats {
    pub open_tasks: i64,
    pub claimed_tasks: i64,
    pub active_leases: i64,
    pub expired_leases: i64,
    pub unread_total: i64,
    pub unread_by_agent: std::collections::BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EventView {
    pub id: i64,
    pub kind: String,
    pub summary: String,
    pub agent_name: Option<String>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<i64>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ListEvents {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub before_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OverviewSession {
    #[serde(flatten)]
    pub session: SessionView,
    pub unread: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskColumns {
    pub open: Vec<TaskView>,
    pub claimed: Vec<TaskView>,
    pub done: Vec<TaskView>,
    pub cancelled: Vec<TaskView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Thresholds {
    pub active_seconds: i64,
    pub idle_seconds: i64,
    pub lease_seconds: i64,
    pub auto_end_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Overview {
    pub generated_at: Timestamp,
    pub operator_name: String,
    pub thresholds: Thresholds,
    /// Live sessions by presence (`active`, `idle`, `working`, `offline`).
    pub presence_counts: std::collections::BTreeMap<String, i64>,
    /// Live sessions plus sessions ended in the last day, newest first.
    pub sessions: Vec<OverviewSession>,
    pub tasks: TaskColumns,
    pub stats: Stats,
    pub blocked_by_cancelled: i64,
    pub events: Vec<EventView>,
    pub decisions: Vec<DecisionView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Health {
    pub ok: bool,
    pub schema_version: i64,
}

// ---------- export ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    /// Directory inside the export repository. Default `coordination`.
    #[serde(default)]
    pub subdir: Option<String>,
    /// Also `git add` + `git commit` exactly the exported paths.
    #[serde(default)]
    pub commit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportResult {
    /// Repo-relative paths written.
    pub paths: Vec<String>,
    /// Repo-relative paths removed as stale.
    pub removed: Vec<String>,
    pub committed: bool,
}

// ---------- work sources (plan 0008) ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SourceSummary {
    pub source: corvus_domain::SourceId,
    pub enabled: bool,
    pub site: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncStats {
    pub tickets: usize,
    pub tasks_created: usize,
    pub comments_delivered: usize,
    pub followups: usize,
    pub resumed: usize,
    pub out_of_scope: usize,
}

/// `POST /api/sources/{source}/sync`: a failed pass is reported here (and in `last_error`), not as an HTTP error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncResult {
    pub ok: bool,
    pub stats: Option<SyncStats>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SourceTicketView {
    pub key: String,
    pub project: String,
    pub summary: String,
    pub status: String,
    pub status_category: corvus_domain::StatusCategory,
    pub priority: i64,
    /// `https:` only (empty otherwise); the dashboard opens it in the system browser.
    pub url: String,
    pub resolved_at: Option<Timestamp>,
    pub assignee: Option<String>,
    /// A rule covers this ticket.
    pub auto_queue: bool,
    /// The live link was queued by hand.
    pub manual: bool,
    pub task_id: Option<String>,
    pub task_status: Option<TaskStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SourceView {
    pub source: corvus_domain::SourceId,
    pub enabled: bool,
    pub site: Option<String>,
    pub filter: Option<corvus_domain::SourceFilter>,
    pub tickets: Vec<SourceTicketView>,
    pub known_statuses: Vec<String>,
    pub last_sync_at: Option<Timestamp>,
    pub last_stats: Option<SyncStats>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct QueueTicket {
    pub key: String,
}
