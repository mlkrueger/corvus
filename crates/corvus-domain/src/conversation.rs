//! A tool's own conversation, used as an agent's identity ([D0023]).
//!
//! [D0023]: ../../../spec/decisions/0023-agent-identity-follows-conversation.md

use crate::enums::Tool;
use crate::error::{CorvusError, Result};

/// Longest accepted conversation id.
pub const MAX_CONVERSATION_ID_LEN: usize = 128;

/// A validated `(tool, conversation id)` pair. The id is safe to use in SQL parameters and file names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationRef {
    tool: Tool,
    id: String,
}

impl ConversationRef {
    /// Accepts ids of 1 to 128 characters from `[A-Za-z0-9_.-]` that are not `.` or `..`.
    pub fn new(tool: Tool, id: &str) -> Result<Self> {
        let ok = !id.is_empty()
            && id.len() <= MAX_CONVERSATION_ID_LEN
            && id != "."
            && id != ".."
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
        if !ok {
            return Err(CorvusError::Validation(
                "conversation id: 1 to 128 characters from A-Z a-z 0-9 _ . -".into(),
            ));
        }
        Ok(Self {
            tool,
            id: id.to_owned(),
        })
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_ids_the_tools_use() {
        for id in [
            "48ab6180-bde1-4698-b7a6-ac1a9e20db89",
            "ses_e1ae272a6ffel20y2qkVLnxWQO",
            "a",
            &"x".repeat(128),
        ] {
            let c = ConversationRef::new(Tool::ClaudeCode, id).unwrap();
            assert_eq!(c.id(), id);
            assert_eq!(c.tool(), Tool::ClaudeCode);
        }
    }

    #[test]
    fn rejects_anything_that_could_escape_a_path_or_surprise_a_query() {
        for id in [
            "",
            ".",
            "..",
            "a/b",
            "../x",
            "a b",
            "a\0b",
            "a'b",
            "é",
            &"x".repeat(129),
        ] {
            assert!(ConversationRef::new(Tool::OpenCode, id).is_err(), "{id:?}");
        }
    }
}
