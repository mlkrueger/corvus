//! The one error type shared by every Corvus transport.

use thiserror::Error;

/// Domain-level error. Transports map [`CorvusError::code`] and [`CorvusError::exit_code`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CorvusError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    Validation(String),
    #[error("session is not active; call register_session again")]
    SessionNotActive,
    #[error("unauthorized")]
    Unauthorized,
    #[error("database is busy; retry")]
    Busy,
    /// Another process migrated the database after this one opened it.
    #[error(
        "Corvus was upgraded — restart this process (database schema {database}, this process expects {expected})"
    )]
    Upgraded { database: i64, expected: i64 },
    #[error("internal error: {0}")]
    Internal(String),
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, CorvusError>;

impl CorvusError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Validation(_) => "validation",
            Self::SessionNotActive => "session_not_active",
            Self::Unauthorized => "unauthorized",
            Self::Busy => "busy",
            Self::Upgraded { .. } => "upgraded",
            Self::Internal(_) => "internal",
        }
    }

    /// CLI process exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::NotFound(_) => 4,
            Self::Conflict(_) | Self::SessionNotActive => 5,
            Self::Validation(_) => 2,
            Self::Unauthorized => 3,
            Self::Busy => 6,
            Self::Upgraded { .. } => 7,
            Self::Internal(_) => 1,
        }
    }

    pub(crate) fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_exit_codes() {
        let cases = [
            (CorvusError::NotFound("x".into()), "not_found", 4),
            (CorvusError::Conflict("x".into()), "conflict", 5),
            (CorvusError::Validation("x".into()), "validation", 2),
            (CorvusError::SessionNotActive, "session_not_active", 5),
            (CorvusError::Unauthorized, "unauthorized", 3),
            (CorvusError::Busy, "busy", 6),
            (
                CorvusError::Upgraded {
                    database: 7,
                    expected: 6,
                },
                "upgraded",
                7,
            ),
            (CorvusError::Internal("x".into()), "internal", 1),
        ];
        for (e, code, exit) in cases {
            assert_eq!(e.code(), code);
            assert_eq!(e.exit_code(), exit);
        }
    }

    #[test]
    fn fixed_messages() {
        assert_eq!(
            CorvusError::SessionNotActive.to_string(),
            "session is not active; call register_session again"
        );
        assert_eq!(CorvusError::Busy.to_string(), "database is busy; retry");
        assert_eq!(
            CorvusError::Upgraded {
                database: 7,
                expected: 6
            }
            .to_string(),
            "Corvus was upgraded — restart this process (database schema 7, this process expects 6)"
        );
    }
}
