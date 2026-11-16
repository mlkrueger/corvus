//! `corvus setup claude` ([system/06](../../../../spec/system/06-delivery-helpers.md#claude-code-hook-corvus-hook)):
//! the pure part — Claude Code settings text in, settings text out. No IO here; the CLI reads, asks and writes.
//!
//! Only Corvus's own hook entries are touched. They are recognised by their command: a program named `corvus`
//! followed by `hook` or `hook wait`, with nothing in front (a hand-written `CORVUS_AGENT_NAME=x corvus hook`
//! is someone else's entry and stays). Key order is kept, so the rest of the file comes back unchanged apart
//! from being printed with two-space indentation.

use corvus_domain::{CorvusError, Result};
use std::fmt;
use std::path::Path;

/// Events that run the inbox hook, in the order they are written to a new file.
pub const HOOK_EVENTS: [&str; 5] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PostToolUse",
    "Stop",
];
/// Events that also start the waiter.
pub const WAIT_EVENTS: [&str; 2] = ["SessionStart", "Stop"];
/// Claude Code kills an async hook after 600 s by default; the waiter has to outlive an idle day.
pub const WAIT_TIMEOUT_SECONDS: u32 = 86_400;
/// The `permissions.allow` rule that pre-approves every tool of an MCP server named `corvus`.
pub const ALLOW_RULE: &str = "mcp__corvus";

/// JSON with object keys in file order (the workspace's `serde_json` sorts them).
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl<'de> serde::Deserialize<'de> for Json {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Json;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> std::result::Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<Json, E> {
                Ok(Json::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<Json, E> {
                Ok(Json::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<Json, E> {
                Ok(Json::Num(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Json, E> {
                serde_json::Number::from_f64(v)
                    .map(Json::Num)
                    .ok_or_else(|| E::custom("a number that is not finite"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Json, E> {
                Ok(Json::Str(v.to_owned()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Json, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = seq.next_element()? {
                    out.push(v);
                }
                Ok(Json::Arr(out))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Json, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Json>()? {
                    out.push((k, v));
                }
                Ok(Json::Obj(out))
            }
        }
        de.deserialize_any(V)
    }
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn write(&self, out: &mut String, depth: usize) {
        let pad = |out: &mut String, depth: usize| out.push_str(&"  ".repeat(depth));
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&n.to_string()),
            Json::Str(s) => out.push_str(&quote(s)),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Obj(entries) if entries.is_empty() => out.push_str("{}"),
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    pad(out, depth + 1);
                    item.write(out, depth + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push(']');
            }
            Json::Obj(entries) => {
                out.push_str("{\n");
                for (i, (k, v)) in entries.iter().enumerate() {
                    pad(out, depth + 1);
                    out.push_str(&quote(k));
                    out.push_str(": ");
                    v.write(out, depth + 1);
                    out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }

    /// The same value with object keys sorted, for comparing without regard to key order.
    fn sorted(&self) -> Json {
        match self {
            Json::Arr(items) => Json::Arr(items.iter().map(Json::sorted).collect()),
            Json::Obj(entries) => {
                let mut out: Vec<(String, Json)> =
                    entries.iter().map(|(k, v)| (k.clone(), v.sorted())).collect();
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Json::Obj(out)
            }
            other => other.clone(),
        }
    }

    fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }
}

fn invalid(msg: impl Into<String>) -> CorvusError {
    CorvusError::Validation(msg.into())
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

fn obj(entries: &[(&str, Json)]) -> Json {
    Json::Obj(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

/// Which Corvus hook a command line runs, if it is one of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ours {
    Hook,
    Wait,
}

fn ours(command: &str) -> Option<Ours> {
    let command = command.trim();
    let (program, rest) = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"')?
    } else {
        command.split_once(char::is_whitespace)?
    };
    let name = Path::new(program).file_name()?.to_str()?;
    if name != "corvus" && name != "corvus.exe" {
        return None;
    }
    match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["hook"] => Some(Ours::Hook),
        ["hook", "wait"] => Some(Ours::Wait),
        _ => None,
    }
}

fn handler_is_ours(handler: &Json) -> bool {
    matches!(handler.get("command"), Some(Json::Str(c)) if ours(c).is_some())
}

/// The hook command for a `corvus` binary at `exe`: the absolute path, double-quoted when it needs it.
pub fn hook_program(exe: &Path) -> Result<String> {
    let text = exe
        .to_str()
        .ok_or_else(|| invalid("the path of the corvus binary is not valid UTF-8"))?;
    if text
        .chars()
        .any(|c| matches!(c, '"' | '$' | '`' | '\\' | '\n' | '!'))
    {
        return Err(invalid(format!(
            "the path of the corvus binary cannot be written safely into a hook command: {text}"
        )));
    }
    let plain = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':'));
    Ok(if plain {
        text.to_owned()
    } else {
        format!("\"{text}\"")
    })
}

fn wanted(event: &str, program: &str) -> Vec<Json> {
    let mut out = Vec::new();
    if HOOK_EVENTS.contains(&event) {
        out.push(obj(&[
            ("type", Json::Str("command".into())),
            ("command", Json::Str(format!("{program} hook"))),
        ]));
    }
    if WAIT_EVENTS.contains(&event) {
        out.push(obj(&[
            ("type", Json::Str("command".into())),
            ("command", Json::Str(format!("{program} hook wait"))),
            ("asyncRewake", Json::Bool(true)),
            ("timeout", Json::Num(WAIT_TIMEOUT_SECONDS.into())),
        ]));
    }
    out
}

fn group(event: &str, handlers: Vec<Json>) -> Json {
    let mut entries = Vec::new();
    if event == "PostToolUse" {
        entries.push(("matcher".to_owned(), Json::Str("*".into())));
    }
    entries.push(("hooks".to_owned(), Json::Arr(handlers)));
    Json::Obj(entries)
}

fn shape(path: &str) -> CorvusError {
    invalid(format!(
        "the settings file has an unexpected shape at {path}; nothing was changed"
    ))
}

/// The handler lists of an event's groups, checked for shape.
fn groups_mut<'a>(event: &str, value: &'a mut Json) -> Result<Vec<&'a mut Vec<Json>>> {
    let Json::Arr(groups) = value else {
        return Err(shape(&format!("hooks.{event}")));
    };
    let mut out = Vec::new();
    for g in groups {
        let Json::Obj(entries) = g else {
            return Err(shape(&format!("hooks.{event}[]")));
        };
        for (k, v) in entries {
            if k == "hooks" {
                let Json::Arr(handlers) = v else {
                    return Err(shape(&format!("hooks.{event}[].hooks")));
                };
                out.push(handlers);
            }
        }
    }
    Ok(out)
}

/// Brings one event to `want` (empty = no Corvus entries). Returns whether anything changed.
fn settle_event(event: &str, value: &mut Json, want: &[Json]) -> Result<bool> {
    // Already exactly right — in one group, in order, with a matcher that matches everything: leave it alone.
    let mut found: Vec<Json> = Vec::new();
    let mut spread = 0;
    if let Json::Arr(groups) = &*value {
        for g in groups {
            let mine: Vec<Json> = match g.get("hooks") {
                Some(Json::Arr(handlers)) => {
                    handlers.iter().filter(|h| handler_is_ours(h)).cloned().collect()
                }
                _ => Vec::new(),
            };
            if !mine.is_empty() {
                let catch_all = matches!(g.get("matcher"), None | Some(Json::Str(_)))
                    && !matches!(g.get("matcher"), Some(Json::Str(m)) if !m.is_empty() && m != "*");
                spread += if catch_all { 1 } else { 2 };
                found.extend(mine);
            }
        }
    }
    let same = found.iter().map(Json::sorted).eq(want.iter().map(Json::sorted));
    if same && (want.is_empty() || spread == 1) {
        // Still validate the shape so a later write never rests on a guess.
        groups_mut(event, value)?;
        return Ok(false);
    }
    for handlers in groups_mut(event, value)? {
        handlers.retain(|h| !handler_is_ours(h));
    }
    if let Json::Arr(groups) = value {
        // Drop groups this left empty (only ours were in them); groups that were already empty stay.
        groups.retain(|g| !matches!(g.get("hooks"), Some(Json::Arr(h)) if h.is_empty()) || found.is_empty());
        if !want.is_empty() {
            groups.push(group(event, want.to_vec()));
        }
    }
    Ok(true)
}

/// What `corvus setup claude` would do to a settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The current content, printed the way `after` is (empty when there is no file).
    pub before: String,
    /// The content to write.
    pub after: String,
    /// False when the file already is as wanted: nothing to write.
    pub changed: bool,
}

/// Plans the edit. `original` is the file's text (`None`: no file). `program` comes from [`hook_program`].
/// With `remove`, every Corvus entry is taken out, and the containers that leaves empty with them.
pub fn plan(original: Option<&str>, program: &str, remove: bool) -> Result<Change> {
    let blank = original.is_none_or(|t| t.trim().is_empty());
    let mut root: Json = if blank {
        Json::Obj(Vec::new())
    } else {
        serde_json::from_str(original.unwrap_or_default()).map_err(|e| {
            invalid(format!(
                "the settings file is not valid JSON ({e}); nothing was changed"
            ))
        })?
    };
    let before = if blank { String::new() } else { root.pretty() };
    let Json::Obj(top) = &mut root else {
        return Err(shape("the top level (expected an object)"));
    };

    let mut changed = false;
    if !top.iter().any(|(k, _)| k == "hooks") && !remove {
        top.push(("hooks".to_owned(), Json::Obj(Vec::new())));
    }
    if let Some((_, hooks)) = top.iter_mut().find(|(k, _)| k == "hooks") {
        let Json::Obj(events) = hooks else {
            return Err(shape("hooks (expected an object)"));
        };
        for (event, value) in events.iter_mut() {
            let want = if remove {
                Vec::new()
            } else {
                wanted(event, program)
            };
            changed |= settle_event(event, value, &want)?;
        }
        if !remove {
            for event in HOOK_EVENTS {
                if !events.iter().any(|(k, _)| k == event) {
                    let handlers = wanted(event, program);
                    events.push((event.to_owned(), Json::Arr(vec![group(event, handlers)])));
                    changed = true;
                }
            }
        }
        if changed {
            events.retain(|(_, v)| !matches!(v, Json::Arr(groups) if groups.is_empty()));
        }
    }
    if changed {
        top.retain(|(k, v)| !(k == "hooks" && matches!(v, Json::Obj(e) if e.is_empty())));
    }
    let after = if changed {
        root.pretty()
    } else {
        original.unwrap_or_default().to_owned()
    };
    Ok(Change {
        before,
        after,
        changed,
    })
}

/// The hooks block on its own, for pasting by hand (docs, the app's "Connect an agent" view).
pub fn snippet(program: &str) -> String {
    plan(None, program, false).map(|c| c.after).unwrap_or_default()
}

/// A line diff of `before` → `after`: changed lines marked `+`/`-`, with two lines of context.
pub fn diff(before: &str, after: &str) -> String {
    const CONTEXT: usize = 2;
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    // Longest common subsequence table (settings files are small).
    let mut lcs = vec![vec![0_u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut lines: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            lines.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] > lcs[i + 1][j]) {
            lines.push(('+', b[j]));
            j += 1;
        } else {
            lines.push(('-', a[i]));
            i += 1;
        }
    }
    let keep: Vec<bool> = (0..lines.len())
        .map(|n| {
            let from = n.saturating_sub(CONTEXT);
            let to = (n + CONTEXT + 1).min(lines.len());
            lines[from..to].iter().any(|(mark, _)| *mark != ' ')
        })
        .collect();
    let mut out = String::new();
    let mut skipped = false;
    for (n, (mark, text)) in lines.iter().enumerate() {
        if keep[n] {
            if skipped && !out.is_empty() {
                out.push_str("  ...\n");
            }
            skipped = false;
            out.push(*mark);
            out.push(' ');
            out.push_str(text);
            out.push('\n');
        } else {
            skipped = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "/opt/corvus/bin/corvus";

    fn parsed(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("json")
    }

    #[test]
    fn fresh_file_gets_every_entry() {
        let c = plan(None, P, false).expect("plan");
        assert!(c.changed);
        assert_eq!(c.before, "");
        let v = parsed(&c.after);
        for event in HOOK_EVENTS {
            let handlers = v["hooks"][event][0]["hooks"].as_array().expect(event);
            assert_eq!(handlers[0]["command"], format!("{P} hook"));
            assert_eq!(handlers[0]["type"], "command");
            assert_eq!(
                handlers.len(),
                if WAIT_EVENTS.contains(&event) { 2 } else { 1 },
                "{event}"
            );
        }
        for event in WAIT_EVENTS {
            let wait = &v["hooks"][event][0]["hooks"][1];
            assert_eq!(wait["command"], format!("{P} hook wait"));
            assert_eq!(wait["asyncRewake"], true);
            assert_eq!(wait["timeout"], 86_400);
        }
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert!(v["hooks"]["Stop"][0].get("matcher").is_none());
        assert!(c.after.ends_with("}\n"));
        // An empty or whitespace-only file is treated like a missing one.
        assert_eq!(plan(Some("  \n"), P, false).expect("plan").after, c.after);
        assert_eq!(snippet(P), c.after);
    }

    const EXISTING: &str = r#"{
  "model": "opus",
  "zeta": [1, 2.5, -3, null, true],
  "hooks": {
    "Stop": [
      { "hooks": [{ "type": "command", "command": "say done" }] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "CORVUS_AGENT_NAME=me corvus hook" }] }
    ]
  },
  "alpha": { "nested": "kept \"quoted\" é" }
}"#;

    #[test]
    fn unrelated_content_and_key_order_are_kept() {
        let c = plan(Some(EXISTING), P, false).expect("plan");
        assert!(c.changed);
        let v = parsed(&c.after);
        let mut expected = parsed(EXISTING);
        // Everything that was there is still there.
        assert_eq!(v["model"], expected["model"]);
        assert_eq!(v["zeta"], expected["zeta"]);
        assert_eq!(v["alpha"], expected["alpha"]);
        assert_eq!(v["hooks"]["PreToolUse"], expected["hooks"]["PreToolUse"]);
        assert_eq!(v["hooks"]["Stop"][0], expected["hooks"]["Stop"][0]);
        assert_eq!(
            v["hooks"]["Stop"][1]["hooks"][1]["command"],
            format!("{P} hook wait")
        );
        // Top-level keys keep their order (not sorted), and so do the existing events.
        let pos = |needle: &str| c.after.find(needle).expect(needle);
        assert!(pos("\"model\"") < pos("\"zeta\""));
        assert!(pos("\"zeta\"") < pos("\"hooks\""));
        assert!(pos("\"hooks\"") < pos("\"alpha\""));
        assert!(pos("\"Stop\"") < pos("\"PreToolUse\""));
        assert!(pos("\"PreToolUse\"") < pos("\"SessionStart\""));

        // Removing again gives back exactly what was there.
        let back = plan(Some(&c.after), P, true).expect("plan");
        assert!(back.changed);
        assert_eq!(parsed(&back.after), expected.take());
        assert_eq!(back.after, c.before);
    }

    #[test]
    fn second_run_changes_nothing() {
        let first = plan(Some(EXISTING), P, false).expect("plan");
        let second = plan(Some(&first.after), P, false).expect("plan");
        assert!(!second.changed);
        assert_eq!(second.after, first.after);
        // Not even the formatting of a hand-formatted file that is already right.
        let compact = serde_json::to_string(&parsed(&first.after)).expect("json");
        let third = plan(Some(&compact), P, false).expect("plan");
        assert!(!third.changed);
        assert_eq!(third.after, compact);
    }

    #[test]
    fn a_moved_binary_replaces_the_old_entries() {
        let old = plan(None, "/old/place/corvus", false).expect("plan");
        let c = plan(Some(&old.after), "\"/new place/corvus\"", false).expect("plan");
        assert!(c.changed);
        assert!(!c.after.contains("/old/place"));
        let v = parsed(&c.after);
        for event in HOOK_EVENTS {
            assert_eq!(v["hooks"][event].as_array().expect(event).len(), 1, "{event}");
        }
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            "\"/new place/corvus\" hook"
        );
        assert!(
            !plan(Some(&c.after), "\"/new place/corvus\"", false)
                .expect("plan")
                .changed
        );
    }

    #[test]
    fn remove_takes_only_corvus_entries() {
        // Ours shares a group with someone else's handler: the group and the handler stay.
        let mixed = format!(
            r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"{P} hook"}},{{"type":"command","command":"say done"}}]}}],"Notification":[]}},"x":1}}"#
        );
        let c = plan(Some(&mixed), P, true).expect("plan");
        assert!(c.changed);
        assert_eq!(
            parsed(&c.after),
            parsed(r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"x":1}"#)
        );
        // Only ours: the hooks key goes too.
        let only = plan(None, P, false).expect("plan");
        let c = plan(Some(&only.after), P, true).expect("plan");
        assert_eq!(c.after, "{}\n");
        // Nothing of ours: nothing to do, with or without a file.
        assert!(!plan(Some(EXISTING), P, true).expect("plan").changed);
        assert!(!plan(None, P, true).expect("plan").changed);
        assert!(!plan(Some(r#"{"hooks":{}}"#), P, true).expect("plan").changed);
    }

    #[test]
    fn invalid_or_unexpected_files_are_refused() {
        for bad in [
            "{ not json",
            "{\"a\": 1} trailing",
            "// comment\n{}",
            "[]",
            "\"text\"",
            r#"{"hooks": []}"#,
            r#"{"hooks": {"Stop": {}}}"#,
            r#"{"hooks": {"Stop": ["x"]}}"#,
            r#"{"hooks": {"Stop": [{"hooks": "x"}]}}"#,
        ] {
            for remove in [false, true] {
                let e = plan(Some(bad), P, remove).expect_err(bad);
                assert_eq!(e.code(), "validation", "{bad}");
                assert!(e.to_string().contains("nothing was changed"), "{bad}: {e}");
            }
        }
    }

    #[test]
    fn recognises_only_its_own_commands() {
        assert_eq!(ours("corvus hook"), Some(Ours::Hook));
        assert_eq!(ours("  /usr/local/bin/corvus  hook  wait "), Some(Ours::Wait));
        assert_eq!(ours("\"/Applications/My Apps/corvus\" hook"), Some(Ours::Hook));
        for other in [
            "CORVUS_AGENT_NAME=me corvus hook",
            "corvus hook wait --x",
            "corvus watch me",
            "corvus",
            "/usr/bin/corvus-old hook",
            "\"unterminated hook",
            "echo corvus hook",
            "",
        ] {
            assert_eq!(ours(other), None, "{other}");
        }
    }

    #[test]
    fn program_is_quoted_only_when_needed() {
        assert_eq!(
            hook_program(Path::new("/usr/local/bin/corvus")).expect("ok"),
            "/usr/local/bin/corvus"
        );
        let spaced = hook_program(Path::new("/Applications/Corvus App/corvus")).expect("ok");
        assert_eq!(spaced, "\"/Applications/Corvus App/corvus\"");
        assert_eq!(ours(&format!("{spaced} hook wait")), Some(Ours::Wait));
        assert!(hook_program(Path::new("/tmp/$HOME/corvus")).is_err());
        assert!(hook_program(Path::new("/tmp/a\"b/corvus")).is_err());
    }

    #[test]
    fn a_wrong_matcher_or_split_entries_are_rebuilt() {
        let split = format!(
            r#"{{"hooks":{{"PostToolUse":[{{"matcher":"Bash","hooks":[{{"type":"command","command":"{P} hook"}}]}}]}}}}"#
        );
        let c = plan(Some(&split), P, false).expect("plan");
        let v = parsed(&c.after);
        assert_eq!(v["hooks"]["PostToolUse"].as_array().expect("array").len(), 1);
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
    }

    #[test]
    fn diff_marks_only_what_changed() {
        let d = diff("a\nb\nc\nd\ne\nf\ng\n", "a\nb\nc\nd\nX\nf\ng\nh\n");
        assert_eq!(d, "  c\n  d\n- e\n+ X\n  f\n  g\n+ h\n");
        assert_eq!(diff("same\n", "same\n"), "");
        assert_eq!(diff("", "new\n"), "+ new\n");
        let c = plan(Some(EXISTING), P, false).expect("plan");
        let d = diff(&c.before, &c.after);
        assert!(d.lines().any(|l| l.starts_with("+ ") && l.contains("hook wait")));
        assert!(!d.lines().any(|l| l.starts_with("- ")), "{d}");
        assert!(!d.contains("\"model\""));
    }
}
