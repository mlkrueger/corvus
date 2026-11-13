//! String-backed enums matching the SQL CHECK constraints exactly.

use crate::error::{CorvusError, Result};
use std::fmt;
use std::str::FromStr;

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident, $label:literal, { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($variant),+ }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(&self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CorvusError;
            fn from_str(s: &str) -> Result<Self> {
                match s {
                    $($s => Ok($name::$variant),)+
                    _ => Err(CorvusError::Validation(format!("unknown {}: {:?}", $label, s))),
                }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
                ser.serialize_str(self.as_str())
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "enum": [$($s),+] })
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(de)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

string_enum!(/// Task lifecycle status.
    TaskStatus, "task status", {
    Open => "open", Claimed => "claimed", Done => "done", Cancelled => "cancelled"
});

string_enum!(/// Message type.
    MessageType, "message type", {
    Note => "note", Question => "question", Answer => "answer",
    Handoff => "handoff", Status => "status", Decision => "decision"
});

string_enum!(/// Why a claim ended.
    EndReason, "claim end reason", {
    Released => "released", Completed => "completed", Expired => "expired", Cancelled => "cancelled"
});

string_enum!(/// Why a session ended.
    SessionEndReason, "session end reason", {
    Ended => "ended", AutoEnded => "auto_ended"
});

string_enum!(/// Artifact kind.
    ArtifactKind, "artifact kind", {
    File => "file", Branch => "branch", Worktree => "worktree", Commit => "commit", Pr => "pr"
});

string_enum!(/// Audit event kind (heartbeats are deliberately not audited).
    EventKind, "event kind", {
    SessionRegistered => "session.registered",
    SessionEnded => "session.ended",
    SessionAutoEnded => "session.auto_ended",
    TaskCreated => "task.created",
    TaskClaimed => "task.claimed",
    TaskRenewed => "task.renewed",
    TaskReleased => "task.released",
    TaskCompleted => "task.completed",
    TaskCancelled => "task.cancelled",
    TaskAppended => "task.appended",
    LeaseExpired => "lease.expired",
    ThreadCreated => "thread.created",
    MessagePosted => "message.posted",
    InboxAcked => "inbox.acked",
    DigestWritten => "digest.written",
    DecisionRecorded => "decision.recorded",
    ArtifactAttached => "artifact.attached",
    SourceTaskCreated => "source.task_created",
    SourceCommentRouted => "source.comment_routed",
    SourceOutOfScope => "source.out_of_scope",
    SourceSyncFailed => "source.sync_failed",
    ExportWritten => "export.written",
    AdminBackup => "admin.backup"
});

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    macro_rules! enum_tests {
        ($($t:ident => $m:ident),*) => {$(
            mod $m {
                use super::*;

                #[test]
                fn roundtrip_all() {
                    assert!(!$t::ALL.is_empty());
                    for v in $t::ALL {
                        assert_eq!($t::from_str(v.as_str()).unwrap(), *v);
                        assert_eq!(v.to_string(), v.as_str());
                        let json = serde_json::to_string(v).unwrap();
                        assert_eq!(json, format!("\"{}\"", v.as_str()));
                        assert_eq!(serde_json::from_str::<$t>(&json).unwrap(), *v);
                    }
                    let mut names: Vec<_> = $t::ALL.iter().map(|v| v.as_str()).collect();
                    names.sort_unstable();
                    names.dedup();
                    assert_eq!(names.len(), $t::ALL.len());
                }

                proptest! {
                    #[test]
                    fn unknown_rejected(s in ".{0,20}") {
                        let known = $t::ALL.iter().any(|v| v.as_str() == s);
                        let r = $t::from_str(&s);
                        prop_assert_eq!(r.is_ok(), known);
                        if let Err(e) = r {
                            prop_assert_eq!(e.code(), "validation");
                        }
                        let quoted = serde_json::to_string(&s).unwrap();
                        prop_assert_eq!(serde_json::from_str::<$t>(&quoted).is_ok(), known);
                    }
                }
            }
        )*};
    }

    enum_tests!(TaskStatus => task_status, MessageType => message_type, EndReason => end_reason,
        SessionEndReason => session_end_reason, ArtifactKind => artifact_kind, EventKind => event_kind);

    #[test]
    fn exact_sql_strings() {
        let s = |a: &[ArtifactKind]| a.iter().map(|v| v.as_str()).collect::<Vec<_>>();
        assert_eq!(
            s(ArtifactKind::ALL),
            ["file", "branch", "worktree", "commit", "pr"]
        );
        assert_eq!(EventKind::ALL.len(), 23);
        assert!("Open".parse::<TaskStatus>().is_err());
    }
}
