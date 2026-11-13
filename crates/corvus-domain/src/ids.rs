//! Typed identifiers. UUID ids only wrap/parse; generation lives in `corvus-core`.

use crate::error::{CorvusError, Result};
use std::fmt;
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$m:meta])* $name:ident, $label:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Uuid);

        impl $name {
            pub fn from_uuid(u: Uuid) -> Self {
                Self(u)
            }
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
            /// Parse the canonical hyphenated form (case-insensitive input is normalised).
            pub fn parse(s: &str) -> Result<Self> {
                if s.len() != 36 {
                    return Err(CorvusError::validation(concat!("invalid ", $label, " id")));
                }
                Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|_| CorvusError::validation(concat!("invalid ", $label, " id")))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.hyphenated())
            }
        }

        impl std::str::FromStr for $name {
            type Err = CorvusError;
            fn from_str(s: &str) -> Result<Self> {
                Self::parse(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = CorvusError;
            fn try_from(s: String) -> Result<Self> {
                Self::parse(&s)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.to_string()
            }
        }
    };
}

macro_rules! int_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub fn new(v: i64) -> Self {
                Self(v)
            }
            pub fn get(&self) -> i64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                Self(v)
            }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> i64 {
                v.0
            }
        }
    };
}

uuid_id!(/// Session id.
    SessionId, "session");
uuid_id!(/// Task id.
    TaskId, "task");
uuid_id!(/// Claim id.
    ClaimId, "claim");
uuid_id!(/// Thread id.
    ThreadId, "thread");
uuid_id!(/// Decision id.
    DecisionId, "decision");
uuid_id!(/// Artifact id.
    ArtifactId, "artifact");

int_id!(/// Message id (autoincrement).
    MessageId);
int_id!(/// Digest id (autoincrement).
    DigestId);
int_id!(/// Audit event id (autoincrement).
    EventId);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    macro_rules! uuid_tests {
        ($($t:ident => $m:ident),*) => {$(
            mod $m {
                use super::*;
                proptest! {
                    #[test]
                    fn roundtrip(bits in any::<u128>()) {
                        let u = Uuid::from_u128(bits);
                        let id = $t::from_uuid(u);
                        prop_assert_eq!(id.as_uuid(), u);
                        let s = id.to_string();
                        prop_assert_eq!(s.len(), 36);
                        prop_assert_eq!(&s, &s.to_lowercase());
                        prop_assert_eq!($t::parse(&s).unwrap(), id);
                        let json = serde_json::to_string(&id).unwrap();
                        prop_assert_eq!(&json, &format!("\"{s}\""));
                        prop_assert_eq!(serde_json::from_str::<$t>(&json).unwrap(), id);
                    }

                    #[test]
                    fn garbage_rejected(s in "[^0-9a-fA-F-]{0,40}") {
                        prop_assert!($t::parse(&s).is_err());
                    }
                }

                #[test]
                fn simple_form_rejected() {
                    assert!($t::parse("0190a1b2c3d47e5f8a9b0c1d2e3f4a5b").is_err());
                    assert!(serde_json::from_str::<$t>("\"nope\"").is_err());
                }
            }
        )*};
    }

    uuid_tests!(SessionId => session, TaskId => task, ClaimId => claim,
        ThreadId => thread, DecisionId => decision, ArtifactId => artifact);

    proptest! {
        #[test]
        fn int_ids_roundtrip(v in any::<i64>()) {
            let m = MessageId::new(v);
            prop_assert_eq!(m.get(), v);
            prop_assert_eq!(m.to_string(), v.to_string());
            prop_assert_eq!(serde_json::to_string(&m).unwrap(), v.to_string());
            prop_assert_eq!(serde_json::from_str::<MessageId>(&v.to_string()).unwrap(), m);
            prop_assert_eq!(DigestId::from(v).get(), v);
            prop_assert_eq!(i64::from(EventId::new(v)), v);
        }
    }
}
