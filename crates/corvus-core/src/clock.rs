//! The application clock ([D0007]). Services read `now` once per operation and bind it as a parameter;
//! SQL clock functions are never used in decision logic.
//!
//! [D0007]: ../../../spec/decisions/0007-time-source.md

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// UTC milliseconds since the Unix epoch.
pub type Millis = i64;

/// A source of "now".
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now_ms(&self) -> Millis;
}

/// The operating-system clock, used in production.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> Millis {
        let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
    }
}

/// A clock that only moves when told to. Tests use it instead of sleeping.
#[derive(Debug)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(start_ms: Millis) -> Self {
        Self(AtomicI64::new(start_ms))
    }
    pub fn set(&self, ms: Millis) {
        self.0.store(ms, Ordering::SeqCst);
    }
    pub fn advance_ms(&self, ms: Millis) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
    pub fn advance_secs(&self, secs: i64) {
        self.advance_ms(secs * 1000);
    }
}

impl Default for ManualClock {
    /// 2026-01-01T00:00:00Z — an arbitrary fixed start.
    fn default() -> Self {
        Self::new(1_767_225_600_000)
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> Millis {
        self.0.load(Ordering::SeqCst)
    }
}

/// Formats UTC milliseconds as RFC 3339 with millisecond precision, e.g. `2026-01-01T00:00:00.000Z`.
pub fn format_rfc3339(ms: Millis) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let c = ManualClock::new(1000);
        assert_eq!(c.now_ms(), 1000);
        c.advance_secs(2);
        assert_eq!(c.now_ms(), 3000);
        c.set(5);
        assert_eq!(c.now_ms(), 5);
    }

    #[test]
    fn rfc3339_formatting() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339(1_767_225_600_000), "2026-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(format_rfc3339(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn system_clock_is_after_2025() {
        assert!(SystemClock.now_ms() > 1_735_689_600_000);
    }
}
