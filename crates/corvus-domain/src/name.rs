//! Agent names.

use crate::error::{CorvusError, Result};
use std::fmt;

/// Maximum length of an agent name.
pub const MAX_AGENT_NAME_LEN: usize = 64;

/// Validated agent name: 1-64 chars of `[A-Za-z0-9_.-]`, first char alphanumeric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentName(String);

impl AgentName {
    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() || s.len() > MAX_AGENT_NAME_LEN {
            return Err(CorvusError::validation(format!(
                "agent name must be 1-{MAX_AGENT_NAME_LEN} characters"
            )));
        }
        let mut chars = s.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) {
            return Err(CorvusError::validation(
                "agent name must start with an ASCII letter or digit",
            ));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return Err(CorvusError::validation(
                "agent name may only contain A-Z a-z 0-9 _ . -",
            ));
        }
        Ok(Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for AgentName {
    type Error = CorvusError;
    fn try_from(s: String) -> Result<Self> {
        Self::parse(&s)
    }
}

impl TryFrom<&str> for AgentName {
    type Error = CorvusError;
    fn try_from(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl From<AgentName> for String {
    fn from(n: AgentName) -> String {
        n.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn valid_accepted(s in "[A-Za-z0-9][A-Za-z0-9_.-]{0,63}") {
            let n = AgentName::parse(&s).unwrap();
            prop_assert_eq!(n.as_str(), s.as_str());
            prop_assert_eq!(n.to_string(), s.clone());
            let json = serde_json::to_string(&n).unwrap();
            prop_assert_eq!(&json, &format!("\"{s}\""));
            prop_assert_eq!(serde_json::from_str::<AgentName>(&json).unwrap(), n);
        }

        #[test]
        fn too_long_rejected(s in "[A-Za-z0-9][A-Za-z0-9_.-]{64,100}") {
            prop_assert!(AgentName::parse(&s).is_err());
        }

        #[test]
        fn bad_first_char_rejected(c in prop::sample::select(vec!['_', '.', '-', ' ', '/', 'é']), rest in "[A-Za-z0-9_.-]{0,20}") {
            let s = format!("{c}{rest}");
            prop_assert!(AgentName::parse(&s).is_err());
        }

        #[test]
        fn disallowed_char_rejected(
            pre in "[A-Za-z0-9][A-Za-z0-9_.-]{0,10}",
            bad in prop::sample::select(vec![' ', '/', '\\', ':', '\0', '\n', 'é', '日', '@', '*']),
            post in "[A-Za-z0-9_.-]{0,10}",
        ) {
            let s = format!("{pre}{bad}{post}");
            prop_assert!(AgentName::parse(&s).is_err());
        }
    }

    #[test]
    fn empty_rejected() {
        assert!(AgentName::parse("").is_err());
    }

    #[test]
    fn boundary_lengths() {
        assert!(AgentName::parse(&"a".repeat(64)).is_ok());
        assert!(AgentName::parse(&"a".repeat(65)).is_err());
    }

    #[test]
    fn deserialize_validates() {
        assert!(serde_json::from_str::<AgentName>("\"-bad\"").is_err());
        assert!(serde_json::from_str::<AgentName>("\"\"").is_err());
    }

    #[test]
    fn try_from_impls() {
        assert!(AgentName::try_from("ok").is_ok());
        assert!(AgentName::try_from("-no".to_string()).is_err());
    }
}
