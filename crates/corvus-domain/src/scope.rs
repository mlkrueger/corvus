//! Digest scopes and thread keys.

use crate::error::{CorvusError, Result};
use crate::ids::{TaskId, ThreadId};
use crate::name::AgentName;

/// What a digest (or other scoped record) is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Task(TaskId),
    Thread(ThreadId),
    Agent(AgentName),
}

impl Scope {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Task(_) => "task",
            Self::Thread(_) => "thread",
            Self::Agent(_) => "agent",
        }
    }

    pub fn reference(&self) -> Option<String> {
        match self {
            Self::Global => None,
            Self::Task(id) => Some(id.to_string()),
            Self::Thread(id) => Some(id.to_string()),
            Self::Agent(n) => Some(n.to_string()),
        }
    }

    /// Matches the generated `scope_key` column of `digests`.
    pub fn scope_key(&self) -> String {
        match self.reference() {
            None => "global".to_owned(),
            Some(r) => format!("{}:{}", self.kind(), r),
        }
    }

    pub fn from_parts(kind: &str, reference: Option<&str>) -> Result<Self> {
        match (kind, reference) {
            ("global", None) => Ok(Self::Global),
            ("global", Some(_)) => Err(CorvusError::validation("global scope takes no reference")),
            ("task", Some(r)) => Ok(Self::Task(TaskId::parse(r)?)),
            ("thread", Some(r)) => Ok(Self::Thread(ThreadId::parse(r)?)),
            ("agent", Some(r)) => Ok(Self::Agent(AgentName::parse(r)?)),
            ("task" | "thread" | "agent", None) => Err(CorvusError::validation(format!(
                "{kind} scope requires a reference"
            ))),
            _ => Err(CorvusError::validation(format!("unknown scope kind: {kind:?}"))),
        }
    }
}

/// Prefix reserved for server-derived direct-thread keys.
pub const DIRECT_PREFIX: &str = "direct:";

/// Canonical key of the direct thread between two agents (names sorted).
pub fn direct_thread_key(a: &AgentName, b: &AgentName) -> String {
    let (lo, hi) = if a.as_str() <= b.as_str() { (a, b) } else { (b, a) };
    format!("{DIRECT_PREFIX}{lo}:{hi}")
}

/// Validate a caller-supplied thread key (the `direct:` namespace is reserved).
pub fn validate_thread_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(CorvusError::validation("thread key must not be empty"));
    }
    if key.starts_with(DIRECT_PREFIX) {
        return Err(CorvusError::validation(
            "thread key must not start with 'direct:'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn name() -> impl Strategy<Value = String> {
        "[A-Za-z0-9][A-Za-z0-9_.-]{0,63}"
    }

    proptest! {
        #[test]
        fn scope_roundtrip(bits in any::<u128>(), n in name()) {
            let u = Uuid::from_u128(bits);
            let scopes = [
                Scope::Global,
                Scope::Task(TaskId::from_uuid(u)),
                Scope::Thread(ThreadId::from_uuid(u)),
                Scope::Agent(AgentName::parse(&n).unwrap()),
            ];
            for s in scopes {
                let back = Scope::from_parts(s.kind(), s.reference().as_deref()).unwrap();
                prop_assert_eq!(&back, &s);
                let key = s.scope_key();
                match s.reference() {
                    None => prop_assert_eq!(key, "global"),
                    Some(r) => prop_assert_eq!(key, format!("{}:{}", s.kind(), r)),
                }
            }
        }

        #[test]
        fn global_with_ref_rejected(r in ".{0,20}") {
            prop_assert!(Scope::from_parts("global", Some(&r)).is_err());
        }

        #[test]
        fn non_global_without_ref_rejected(k in prop::sample::select(vec!["task", "thread", "agent"])) {
            prop_assert!(Scope::from_parts(k, None).is_err());
        }

        #[test]
        fn bad_refs_rejected(r in "[^0-9a-f-]{1,20}") {
            prop_assert!(Scope::from_parts("task", Some(&r)).is_err());
            prop_assert!(Scope::from_parts("thread", Some(&r)).is_err());
            let dashed = format!("-{}", r);
            prop_assert!(Scope::from_parts("agent", Some(&dashed)).is_err());
        }

        #[test]
        fn unknown_kind_rejected(k in "[a-z]{1,10}") {
            prop_assume!(!["global", "task", "thread", "agent"].contains(&k.as_str()));
            prop_assert!(Scope::from_parts(&k, None).is_err());
            prop_assert!(Scope::from_parts(&k, Some("x")).is_err());
        }

        #[test]
        fn direct_key_symmetric(a in name(), b in name()) {
            let (a, b) = (AgentName::parse(&a).unwrap(), AgentName::parse(&b).unwrap());
            let k = direct_thread_key(&a, &b);
            prop_assert_eq!(&k, &direct_thread_key(&b, &a));
            let (lo, hi) = if a.as_str() <= b.as_str() { (&a, &b) } else { (&b, &a) };
            prop_assert_eq!(k, format!("direct:{lo}:{hi}"));
        }

        #[test]
        fn thread_key_valid(k in "[^d][ -~]{0,50}") {
            prop_assert!(validate_thread_key(&k).is_ok());
        }

        #[test]
        fn thread_key_direct_rejected(rest in ".{0,30}") {
            let k = format!("direct:{rest}");
            prop_assert!(validate_thread_key(&k).is_err());
        }
    }

    #[test]
    fn empty_thread_key_rejected() {
        assert!(validate_thread_key("").is_err());
        assert!(validate_thread_key("directive").is_ok());
    }
}
