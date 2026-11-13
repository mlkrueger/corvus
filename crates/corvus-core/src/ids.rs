//! Identifier generation. The domain only wraps ids; generating them (time + randomness) is IO-ish and
//! lives here.

use corvus_domain::{ArtifactId, ClaimId, DecisionId, SessionId, TaskId, ThreadId};
use uuid::Uuid;

/// A new UUIDv7.
pub fn new_uuid() -> Uuid {
    Uuid::now_v7()
}

pub fn new_session_id() -> SessionId {
    SessionId::from_uuid(new_uuid())
}
pub fn new_task_id() -> TaskId {
    TaskId::from_uuid(new_uuid())
}
pub fn new_claim_id() -> ClaimId {
    ClaimId::from_uuid(new_uuid())
}
pub fn new_thread_id() -> ThreadId {
    ThreadId::from_uuid(new_uuid())
}
pub fn new_decision_id() -> DecisionId {
    DecisionId::from_uuid(new_uuid())
}
pub fn new_artifact_id() -> ArtifactId {
    ArtifactId::from_uuid(new_uuid())
}
