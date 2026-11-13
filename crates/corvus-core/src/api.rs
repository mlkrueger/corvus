//! Request and response shapes shared by every transport (system/04). One definition of each shape: the HTTP
//! handlers deserialise these, the MCP tools derive their schemas from them, and the CLI builds them.
//!
//! Requests carry plain strings; services validate them into domain types and return `Validation` errors.
//! Responses carry RFC 3339 timestamps.

use corvus_domain::{ArtifactKind, EndReason, Presence, SessionEndReason, TaskStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::clock::Timestamp;

// ---------- sessions ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSessions {
    /// Include ended sessions (default false).
    #[serde(default)]
    pub include_ended: bool,
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndStaleResult {
    /// Ids of the sessions this pass ended.
    pub ended: Vec<String>,
}

// ---------- tasks ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInput {
    pub kind: ArtifactKind,
    pub path: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactView {
    pub id: String,
    pub kind: ArtifactKind,
    pub path: String,
    pub commit: Option<String>,
    pub description: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holder {
    pub agent_name: String,
    pub session_id: String,
    pub claim_id: String,
    pub claimed_at: Timestamp,
    pub lease_expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimTask {
    pub session_id: String,
    #[serde(default)]
    pub pool: Option<String>,
}

/// `claim_task`'s result: both `null` when nothing is claimable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimResult {
    pub task: Option<TaskView>,
    pub claim: Option<ClaimView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRef {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteTask {
    pub session_id: String,
    #[serde(default)]
    pub result_summary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelResult {
    pub task: TaskView,
    /// Ids cancelled by this call (empty on an idempotent replay).
    pub cancelled: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpireResult {
    pub expired: usize,
}
