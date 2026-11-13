//! Presence is derived, never self-reported (brief §4.1). Computed on read, never stored.

/// A session's derived presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Ended,
    Offline,
    Working,
    Active,
    Idle,
}

impl Presence {
    pub const ALL: [Presence; 5] = [
        Presence::Ended,
        Presence::Offline,
        Presence::Working,
        Presence::Active,
        Presence::Idle,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ended => "ended",
            Self::Offline => "offline",
            Self::Working => "working",
            Self::Active => "active",
            Self::Idle => "idle",
        }
    }
}

/// Presence in the order brief §4.1 gives: `ended` if ended; else `offline` if silent *longer* than the idle
/// window; else `working` if it holds an active claim; else `active` if seen *within* the active window
/// (inclusive); else `idle`. Times are UTC milliseconds; windows are milliseconds.
pub fn presence(
    ended: bool,
    last_seen_at: i64,
    has_active_claim: bool,
    now: i64,
    active_window_ms: i64,
    idle_window_ms: i64,
) -> Presence {
    let silent = now.saturating_sub(last_seen_at);
    if ended {
        Presence::Ended
    } else if silent > idle_window_ms {
        Presence::Offline
    } else if has_active_claim {
        Presence::Working
    } else if silent <= active_window_ms {
        Presence::Active
    } else {
        Presence::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Presence::*;

    const A: i64 = 120_000;
    const I: i64 = 900_000;

    #[test]
    fn table() {
        // (ended, silent_ms, claim) -> presence
        let cases: &[(bool, i64, bool, Presence)] = &[
            (true, 0, false, Ended),
            (true, 0, true, Ended),
            (true, I + 1, true, Ended),
            (false, 0, false, Active),
            (false, A, false, Active),
            (false, A + 1, false, Idle),
            (false, I, false, Idle),
            (false, I + 1, false, Offline),
            (false, 0, true, Working),
            (false, A + 1, true, Working),
            (false, I, true, Working),
            (false, I + 1, true, Offline),
            (false, -5_000, false, Active), // clock skew: seen "in the future"
        ];
        let now = 10_000_000;
        for &(ended, silent, claim, want) in cases {
            assert_eq!(
                presence(ended, now - silent, claim, now, A, I),
                want,
                "{ended} {silent} {claim}"
            );
        }
    }

    #[test]
    fn serde_names() {
        for p in Presence::ALL {
            assert_eq!(serde_json_like(p), p.as_str());
        }
    }

    fn serde_json_like(p: Presence) -> &'static str {
        p.as_str()
    }
}
