//! Linear, the pure parts ([system/07b](../../../spec/system/07b-linear.md)): the constant query documents, the
//! GraphQL filter variable, and the vocabulary maps. No IO.
//!
//! The query text is a compile-time constant; every variable value (statuses, team keys, the done-window cutoff,
//! cursors, identifiers) travels in GraphQL `variables`, so no operator or tracker text is ever interpolated.
//! Only queries exist here; there are no mutation documents.

use serde_json::{Value, json};

use crate::sources::{SourceFilter, StatusCategory};

/// Search: `$filter: IssueFilter`, `$cursor: String`.
pub const SEARCH_QUERY: &str = "query Search($filter: IssueFilter, $cursor: String) { \
issues(filter: $filter, first: 100, after: $cursor) { \
nodes { identifier title description url priority updatedAt completedAt \
state { name type } team { key } assignee { name } } \
pageInfo { hasNextPage endCursor } } }";

/// Comments of one issue: `$id: String!` (an identifier such as `ENG-123`), `$cursor: String`.
pub const COMMENTS_QUERY: &str = "query Comments($id: String!, $cursor: String) { \
issue(id: $id) { comments(first: 100, after: $cursor) { \
nodes { id body createdAt user { id name } } \
pageInfo { hasNextPage endCursor } } } }";

/// Workflow state names for the filter form's suggestions.
pub const STATUSES_QUERY: &str = "query Statuses { workflowStates(first: 250) { nodes { name } } }";

/// The default filter: statuses Backlog, Todo, In Progress, In Review; 7 done days; all teams; no auto-queue rules.
pub fn default_filter() -> SourceFilter {
    SourceFilter {
        statuses: ["Backlog", "Todo", "In Progress", "In Review"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        done_days: 7,
        projects: Vec::new(),
        auto_queue: Vec::new(),
    }
}

/// The `$filter` variable for [`SEARCH_QUERY`]. `now_ms` is the engine's clock; the done-window cutoff is
/// `now - done_days` (omitted when `done_days` is 0, and the state clause is omitted when there are no statuses).
pub fn issue_filter(filter: &SourceFilter, now_ms: i64) -> Value {
    let mut or: Vec<Value> = Vec::new();
    if !filter.statuses.is_empty() {
        or.push(json!({ "state": { "name": { "in": filter.statuses } } }));
    }
    if filter.done_days > 0 {
        let cutoff = now_ms.saturating_sub(i64::from(filter.done_days) * 86_400_000);
        or.push(json!({ "completedAt": { "gte": format_rfc3339_ms(cutoff) } }));
    }
    let mut f = serde_json::Map::new();
    f.insert("assignee".into(), json!({ "isMe": { "eq": true } }));
    f.insert("or".into(), Value::Array(or));
    if !filter.projects.is_empty() {
        f.insert("team".into(), json!({ "key": { "in": filter.projects } }));
    }
    Value::Object(f)
}

/// Workflow state `type` to status category. Unknown types are treated as todo.
pub fn state_category(state_type: &str) -> StatusCategory {
    match state_type {
        "started" => StatusCategory::InProgress,
        "completed" | "canceled" => StatusCategory::Done,
        _ => StatusCategory::Todo, // triage, backlog, unstarted
    }
}

/// Linear priority (1 Urgent .. 4 Low, 0 None) to Corvus priority.
pub fn map_priority(p: i64) -> i64 {
    match p {
        1 => 10,
        2 => 5,
        4 => -5,
        _ => 0,
    }
}

/// The URL only if it is `https://linear.app/…` (no whitespace, control characters or backslashes); else empty.
pub fn accept_url(url: &str) -> String {
    let ok = url.starts_with("https://linear.app/")
        && url.len() <= 2048
        && !url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\');
    if ok { url.to_owned() } else { String::new() }
}

// --- time -----------------------------------------------------------------------------------------------------

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Epoch milliseconds to `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_rfc3339_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Parses an RFC 3339 timestamp (`Z` or `±hh:mm`, optional fractional seconds, `T` or space) to epoch
/// milliseconds. Fractions beyond milliseconds are truncated. `None` when malformed.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if !s.is_ascii() || b.len() < 20 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let part = s.get(a..z)?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac_ms = 0;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits = &s[start..i];
        let three: String = digits.chars().chain("000".chars()).take(3).collect();
        frac_ms = three.parse::<i64>().ok()?;
    }
    let offset_min = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1, i + 3)?, num(i + 4, i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let v = oh * 60 + om;
            if *sign == b'-' { -v } else { v }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec.min(59) - offset_min * 60;
    secs.checked_mul(1000)?.checked_add(frac_ms)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const NOW: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

    #[test]
    fn filter_full() {
        let f = SourceFilter {
            statuses: vec!["Todo".into(), "Won't \"fix\"".into()],
            done_days: 7,
            projects: vec!["ENG".into()],
            auto_queue: vec![],
        };
        let v = issue_filter(&f, NOW);
        assert_eq!(
            v,
            json!({
                "assignee": {"isMe": {"eq": true}},
                "or": [
                    {"state": {"name": {"in": ["Todo", "Won't \"fix\""]}}},
                    {"completedAt": {"gte": "2025-12-25T00:00:00.000Z"}}
                ],
                "team": {"key": {"in": ["ENG"]}}
            })
        );
    }

    #[test]
    fn filter_omissions() {
        let f = SourceFilter {
            statuses: vec!["Todo".into()],
            done_days: 0,
            ..SourceFilter::default()
        };
        let v = issue_filter(&f, NOW);
        assert_eq!(v["or"].as_array().unwrap().len(), 1);
        assert!(v.get("team").is_none());
        assert!(v["or"][0].get("completedAt").is_none());
        let g = SourceFilter {
            done_days: 3,
            ..SourceFilter::default()
        };
        let v = issue_filter(&g, NOW);
        assert_eq!(v["or"].as_array().unwrap().len(), 1);
        assert!(v["or"][0].get("completedAt").is_some());
    }

    #[test]
    fn default_is_valid() {
        let d = default_filter();
        d.validate().unwrap();
        assert_eq!(d.done_days, 7);
        assert!(d.auto_queue.is_empty() && d.projects.is_empty());
        assert_eq!(d.statuses, ["Backlog", "Todo", "In Progress", "In Review"]);
    }

    #[test]
    fn maps() {
        for (t, c) in [
            ("triage", StatusCategory::Todo),
            ("backlog", StatusCategory::Todo),
            ("unstarted", StatusCategory::Todo),
            ("started", StatusCategory::InProgress),
            ("completed", StatusCategory::Done),
            ("canceled", StatusCategory::Done),
            ("weird", StatusCategory::Todo),
        ] {
            assert_eq!(state_category(t), c, "{t}");
        }
        let got: Vec<i64> = [0, 1, 2, 3, 4, 9].into_iter().map(map_priority).collect();
        assert_eq!(got, [0, 10, 5, 0, -5, 0]);
    }

    #[test]
    fn url_guard() {
        assert_eq!(
            accept_url("https://linear.app/acme/issue/ENG-1/x"),
            "https://linear.app/acme/issue/ENG-1/x"
        );
        for bad in [
            "",
            "http://linear.app/x",
            "https://linear.app.evil.com/x",
            "https://linear.app@evil.com/x",
            "https://evil.com/https://linear.app/",
            "https://linear.app",
            "https://linear.app/a b",
            "https://linear.app/a\nb",
            "javascript:alert(1)",
        ] {
            assert_eq!(accept_url(bad), "", "{bad}");
        }
    }

    #[test]
    fn time_parsing() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("2026-01-01T00:00:00.000Z"), Some(NOW));
        assert_eq!(parse_rfc3339_ms("2026-01-01T00:00:00.5Z"), Some(NOW + 500));
        assert_eq!(
            parse_rfc3339_ms("2026-01-01T00:00:00.123456789Z"),
            Some(NOW + 123)
        );
        assert_eq!(parse_rfc3339_ms("2026-01-01T01:00:00+01:00"), Some(NOW));
        assert_eq!(parse_rfc3339_ms("2025-12-31T19:00:00-05:00"), Some(NOW));
        assert_eq!(parse_rfc3339_ms("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
        assert_eq!(parse_rfc3339_ms("1969-12-31T23:59:59.999Z"), Some(-1));
        for bad in [
            "",
            "2026-01-01",
            "2026-01-01T00:00:00",
            "2026-13-01T00:00:00Z",
            "2026-01-01T25:00:00Z",
            "2026-01-01T00:00:00.Z",
            "2026-01-01T00:00:00Zjunk",
            "2026-01-01T00:00:00+0100",
            "garbage-garbage-garbage",
            "２026-01-01T00:00:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn time_roundtrip() {
        for ms in [0, 1, NOW, NOW + 999, -1, 951_782_400_000, 4_102_444_800_123] {
            assert_eq!(parse_rfc3339_ms(&format_rfc3339_ms(ms)), Some(ms), "{ms}");
        }
        assert_eq!(format_rfc3339_ms(NOW), "2026-01-01T00:00:00.000Z");
    }

    #[test]
    fn queries_are_read_only_constants() {
        for q in [SEARCH_QUERY, COMMENTS_QUERY, STATUSES_QUERY] {
            assert!(q.starts_with("query "));
            assert!(!q.contains("mutation"));
        }
    }
}
