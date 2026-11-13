//! Small pure helpers: Basic-auth encoding and Jira timestamp parsing.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (only used for the Basic auth header).
pub(crate) fn base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses `2026-01-02T03:04:05.000+0000` (also `Z` and `+hh:mm`) to UTC epoch milliseconds.
pub(crate) fn parse_jira_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (
        num(0, 4)?,
        num(5, 2)?,
        num(8, 2)?,
        num(11, 2)?,
        num(14, 2)?,
        num(17, 2)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut millis = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
        let frac = &s[start..i];
        let three: String = frac.chars().chain("000".chars()).take(3).collect();
        millis = three.parse::<i64>().ok()?;
    }
    let offset_min = match &s[i..] {
        "Z" | "z" => 0,
        rest if rest.len() >= 3 && (rest.starts_with('+') || rest.starts_with('-')) => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let digits: String = rest[1..].chars().filter(|c| *c != ':').collect();
            if !digits.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let (oh, om) = match digits.len() {
                2 => (digits.parse::<i64>().ok()?, 0),
                4 => (digits[..2].parse::<i64>().ok()?, digits[2..].parse::<i64>().ok()?),
                _ => return None,
            };
            sign * (oh * 60 + om)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 24 + h) * 60 + mi - offset_min) * 60_000 + sec * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"user@x.test:tok"), "dXNlckB4LnRlc3Q6dG9r");
    }

    #[test]
    fn times() {
        assert_eq!(parse_jira_time("1970-01-01T00:00:00.000+0000"), Some(0));
        assert_eq!(
            parse_jira_time("2026-01-02T03:04:05.000+0000"),
            Some(1_767_323_045_000)
        );
        assert_eq!(parse_jira_time("2026-01-02T03:04:05Z"), Some(1_767_323_045_000));
        assert_eq!(
            parse_jira_time("2026-01-02T05:04:05.250+02:00"),
            Some(1_767_323_045_250)
        );
        assert_eq!(
            parse_jira_time("2026-01-02T03:04:05.000-0100"),
            Some(1_767_323_045_000 + 3_600_000)
        );
        for bad in [
            "",
            "yesterday",
            "2026-13-02T03:04:05.000+0000",
            "2026-01-02T03:04:05",
            "2026-01-02T03:04:05.+0000",
        ] {
            assert_eq!(parse_jira_time(bad), None, "{bad}");
        }
    }
}
