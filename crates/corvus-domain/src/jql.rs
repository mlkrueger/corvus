//! Jira query building and field maps ([system/07a](../../../spec/system/07a-jira.md)). Pure.
//!
//! The query is built only from the validated [`SourceFilter`]; status names are emitted as escaped, double-quoted
//! JQL strings and project keys (validated `[A-Z][A-Z0-9_]{0,9}`) need no quoting.

use crate::sources::{AutoQueueRule, SourceFilter, StatusCategory};

/// A JQL string literal: double-quoted, `\` and `"` backslash-escaped. Control characters become spaces
/// (validation already rejects them; this keeps the function total).
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The JQL for a filter. Always restricted to the API user's own tickets.
pub fn build_jql(filter: &SourceFilter) -> String {
    let mut q = String::from("assignee = currentUser()");
    let statuses = (!filter.statuses.is_empty()).then(|| {
        let list: Vec<String> = filter.statuses.iter().map(|s| quote(s)).collect();
        format!("status in ({})", list.join(", "))
    });
    let done = (filter.done_days > 0)
        .then(|| format!("(statusCategory = Done AND resolved >= -{}d)", filter.done_days));
    match (statuses, done) {
        (Some(s), Some(d)) => q.push_str(&format!(" AND ( {s} OR {d} )")),
        (Some(s), None) => q.push_str(&format!(" AND {s}")),
        (None, Some(d)) => q.push_str(&format!(" AND {d}")),
        (None, None) => {}
    }
    if !filter.projects.is_empty() {
        q.push_str(&format!(" AND project in ({})", filter.projects.join(", ")));
    }
    q.push_str(" ORDER BY updated DESC");
    q
}

/// Jira priority name to Corvus priority: Highest 10, High 5, Medium 0, Low -5, Lowest -10, anything else 0.
pub fn jira_priority(name: &str) -> i64 {
    match name {
        "Highest" => 10,
        "High" => 5,
        "Medium" => 0,
        "Low" => -5,
        "Lowest" => -10,
        _ => 0,
    }
}

/// Jira `statusCategory.key` to category; unknown keys read as to-do.
pub fn jira_status_category(key: &str) -> StatusCategory {
    match key {
        "indeterminate" => StatusCategory::InProgress,
        "done" => StatusCategory::Done,
        _ => StatusCategory::Todo,
    }
}

/// The Jira tab's default filter.
pub fn default_filter() -> SourceFilter {
    SourceFilter {
        statuses: ["Backlog", "To Do", "In Progress", "Waiting for response"]
            .map(str::to_owned)
            .to_vec(),
        done_days: 7,
        projects: Vec::new(),
        auto_queue: vec![AutoQueueRule {
            projects: vec!["AA".to_owned()],
            statuses: vec!["To Do".to_owned(), "In Progress".to_owned()],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn f(statuses: &[&str], done_days: u32, projects: &[&str]) -> SourceFilter {
        SourceFilter {
            statuses: statuses.iter().map(|s| (*s).to_owned()).collect(),
            done_days,
            projects: projects.iter().map(|s| (*s).to_owned()).collect(),
            auto_queue: vec![],
        }
    }

    #[test]
    fn full_query() {
        assert_eq!(
            build_jql(&f(&["A", "B"], 7, &["AA", "LAI"])),
            "assignee = currentUser() AND ( status in (\"A\", \"B\") OR (statusCategory = Done AND resolved >= -7d) ) AND project in (AA, LAI) ORDER BY updated DESC"
        );
    }

    #[test]
    fn clauses_only_when_set() {
        let q = build_jql(&f(&["A"], 0, &[]));
        assert_eq!(
            q,
            "assignee = currentUser() AND status in (\"A\") ORDER BY updated DESC"
        );
        let q = build_jql(&f(&[], 3, &[]));
        assert_eq!(
            q,
            "assignee = currentUser() AND (statusCategory = Done AND resolved >= -3d) ORDER BY updated DESC"
        );
        assert!(!q.contains("project"));
        assert!(!build_jql(&f(&["A"], 0, &[])).contains("resolved"));
        assert_eq!(
            build_jql(&f(&[], 0, &[])),
            "assignee = currentUser() ORDER BY updated DESC"
        );
    }

    #[test]
    fn escaping() {
        let q = build_jql(&f(&[r#"Won't "fix" \ it"#], 0, &[]));
        assert!(q.contains(r#"status in ("Won't \"fix\" \\ it")"#), "{q}");
    }

    #[test]
    fn maps_and_default() {
        assert_eq!(
            ["Highest", "High", "Medium", "Low", "Lowest", "Blocker", ""].map(jira_priority),
            [10, 5, 0, -5, -10, 0, 0]
        );
        assert_eq!(jira_status_category("new"), StatusCategory::Todo);
        assert_eq!(jira_status_category("indeterminate"), StatusCategory::InProgress);
        assert_eq!(jira_status_category("done"), StatusCategory::Done);
        let d = default_filter();
        d.validate().unwrap();
        assert_eq!(d.done_days, 7);
        assert_eq!(d.auto_queue[0].projects, ["AA"]);
    }

    /// Walks a query, treating `"..."` (with `\` escapes) as opaque literals. Returns the skeleton with literals
    /// replaced by `S`, or `None` if a literal is unterminated or a stray quote/backslash sits outside one.
    fn skeleton(q: &str) -> Option<(String, Vec<String>)> {
        let mut out = String::new();
        let mut lits = Vec::new();
        let mut it = q.chars();
        while let Some(c) = it.next() {
            match c {
                '"' => {
                    let mut lit = String::new();
                    loop {
                        match it.next()? {
                            '\\' => lit.push(it.next()?),
                            '"' => break,
                            c => lit.push(c),
                        }
                    }
                    lits.push(lit);
                    out.push('S');
                }
                '\\' => return None,
                c => out.push(c),
            }
        }
        Some((out, lits))
    }

    proptest! {
        #[test]
        fn status_names_cannot_change_structure(
            names in proptest::collection::vec("[^\\p{Cc}]{1,60}", 1..6),
            done in 0u32..90,
        ) {
            let names: Vec<String> = names.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            prop_assume!(!names.is_empty());
            let filter = SourceFilter { statuses: names.clone(), done_days: done, projects: vec!["AA".into()], auto_queue: vec![] };
            prop_assume!(filter.validate().is_ok());
            let (skel, lits) = skeleton(&build_jql(&filter)).expect("well-formed literals");
            // Literals round-trip exactly, and the skeleton depends only on how many statuses there are.
            prop_assert_eq!(lits, names.clone());
            let reference = SourceFilter { statuses: vec!["x".into(); names.len()], ..filter };
            let (ref_skel, _) = skeleton(&build_jql(&reference)).unwrap();
            prop_assert_eq!(skel, ref_skel);
        }
    }
}
