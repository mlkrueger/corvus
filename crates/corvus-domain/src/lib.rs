//! Pure Corvus domain types and validation. No IO, no clock, no ID generation.

pub mod artifact;
pub mod enums;
pub mod error;
pub mod ids;
pub mod name;
pub mod scope;
pub mod text;

pub use artifact::{ArtifactRef, validate_artifact_path, validate_commit_hash};
pub use enums::{ArtifactKind, EndReason, EventKind, MessageType, SessionEndReason, TaskStatus};
pub use error::{CorvusError, Result};
pub use ids::{ArtifactId, ClaimId, DecisionId, DigestId, EventId, MessageId, SessionId, TaskId, ThreadId};
pub use name::{AgentName, MAX_AGENT_NAME_LEN};
pub use scope::{Scope, direct_thread_key, validate_thread_key};
pub use text::{MAX_BODY_CHARS, validate_message_body, validate_summary, validate_task_body, validate_title};
