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

/// Parses the output of [`format_rfc3339`] (and second-precision `...Z` forms). `None` if malformed.
pub fn parse_rfc3339(s: &str) -> Option<Millis> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let rest = &s[19..];
    let ms = match rest {
        "Z" => 0,
        r if r.len() == 5 && r.starts_with('.') && r.ends_with('Z') => r[1..4].parse::<i64>().ok()?,
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 59 {
        return None;
    }
    let days = days_from_civil(y, mo as u32, d as u32);
    Some(((days * 86_400 + h * 3600 + mi * 60 + se) * 1000) + ms)
}

/// Howard Hinnant's civil-to-days algorithm.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UTC milliseconds that serialise as RFC 3339 (the wire format for every timestamp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ts_rs::TS)]
#[ts(export, type = "string")]
pub struct Timestamp(pub Millis);

impl serde::Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_rfc3339(self.0))
    }
}

impl<'de> serde::Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        parse_rfc3339(&s)
            .map(Timestamp)
            .ok_or_else(|| serde::de::Error::custom("expected an RFC 3339 UTC timestamp"))
    }
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
    fn rfc3339_round_trip() {
        for ms in [0, 1, 999, 951_782_400_123, 1_767_225_600_000, 4_102_444_799_999] {
            assert_eq!(parse_rfc3339(&format_rfc3339(ms)), Some(ms), "{ms}");
        }
        assert_eq!(parse_rfc3339("2026-01-01T00:00:00Z"), Some(1_767_225_600_000));
        for bad in [
            "",
            "2026-01-01",
            "2026-13-01T00:00:00Z",
            "2026-01-01T00:00:00+01:00",
            "2026-01-01T00:00:00.1Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn system_clock_is_after_2025() {
        assert!(SystemClock.now_ms() > 1_735_689_600_000);
    }
}
