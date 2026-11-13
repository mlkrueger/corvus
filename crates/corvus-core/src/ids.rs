//! Identifier generation. The domain only wraps ids; generating them (time + randomness) is IO-ish and
//! lives here.

use corvus_domain::{ArtifactId, ClaimId, DecisionId, SessionId, TaskId, ThreadId};
use uuid::Uuid;

/// 64 random bits (from the OS generator, via a v4 UUID).
pub fn random_u64() -> u64 {
    let b = Uuid::new_v4().into_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[9], b[10]])
}

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
