//! Pure Corvus domain types and validation. No IO, no clock, no ID generation.

pub mod adf;
pub mod artifact;
pub mod conversation;
pub mod enums;
pub mod error;
pub mod guidance;
pub mod ids;
pub mod jql;
pub mod linear;
pub mod name;
pub mod names;
pub mod presence;
pub mod scope;
pub mod sources;
pub mod text;
pub mod wake;

pub use artifact::{ArtifactRef, validate_artifact_path, validate_commit_hash};
pub use conversation::{ConversationRef, MAX_CONVERSATION_ID_LEN};
pub use enums::{ArtifactKind, EndReason, EventKind, MessageType, SessionEndReason, TaskStatus, Tool};
pub use error::{CorvusError, Result};
pub use guidance::GUIDANCE;
pub use ids::{ArtifactId, ClaimId, DecisionId, DigestId, EventId, MessageId, SessionId, TaskId, ThreadId};
pub use name::{AgentName, MAX_AGENT_NAME_LEN};
pub use names::{generated_name, suffixed_name};
pub use presence::{Presence, presence};
pub use scope::{Scope, direct_thread_key, validate_thread_key};
pub use sources::{AutoQueueRule, Comment, SourceFilter, SourceId, StatusCategory, Ticket, TicketKey};
pub use text::{MAX_BODY_CHARS, validate_message_body, validate_summary, validate_task_body, validate_title};
pub use wake::{Adapter, BindingPattern, PLACEHOLDERS, RawAdapter, WAKE_NOTE};
