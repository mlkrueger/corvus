//! Atlassian Document Format to plain text ([system/07a](../../../spec/system/07a-jira.md)). Pure; never panics.

use serde_json::Value;

/// Nesting beyond this depth contributes nothing (guards the stack against hostile input).
const MAX_DEPTH: usize = 40;

/// Flattens an ADF document. A value that is not a `doc` node yields an empty string.
pub fn adf_to_text(doc: &Value) -> String {
    if doc.get("type").and_then(Value::as_str) != Some("doc") {
        return String::new();
    }
    let mut out = String::new();
    blocks(doc.get("content"), 0, &mut out, 0);
    out.trim_end().to_owned()
}

fn kind(n: &Value) -> &str {
    n.get("type").and_then(Value::as_str).unwrap_or("")
}

fn children(n: &Value) -> &[Value] {
    n.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn ensure_newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn blocks(content: Option<&Value>, indent: usize, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in content.and_then(Value::as_array).into_iter().flatten() {
        block(n, indent, out, depth + 1);
    }
}

fn block(n: &Value, indent: usize, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let pad = " ".repeat(indent);
    match kind(n) {
        "paragraph" | "heading" => {
            ensure_newline(out);
            out.push_str(&pad);
            out.push_str(&inline_all(children(n), depth));
            out.push('\n');
        }
        "bulletList" | "orderedList" => {
            let ordered = kind(n) == "orderedList";
            let start = n.pointer("/attrs/order").and_then(Value::as_u64).unwrap_or(1);
            for (i, item) in children(n).iter().enumerate() {
                list_item(
                    item,
                    indent,
                    ordered.then_some(start.saturating_add(i as u64)),
                    out,
                    depth,
                );
            }
        }
        "codeBlock" => {
            ensure_newline(out);
            let lang = n.pointer("/attrs/language").and_then(Value::as_str).unwrap_or("");
            let lang: String = lang
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "+-_#.".contains(*c))
                .collect();
            out.push_str(&format!("{pad}```{lang}\n"));
            let code = inline_all(children(n), depth);
            for line in code.lines() {
                out.push_str(&pad);
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("{pad}```\n"));
        }
        "table" => {
            for row in children(n) {
                let cells: Vec<String> = children(row)
                    .iter()
                    .map(|c| {
                        let mut s = String::new();
                        blocks(c.get("content"), 0, &mut s, depth + 1);
                        s.split_whitespace().collect::<Vec<_>>().join(" ")
                    })
                    .collect();
                ensure_newline(out);
                out.push_str(&pad);
                out.push_str(&cells.join(" | "));
                out.push('\n');
            }
        }
        "mediaSingle" | "mediaGroup" | "media" => {
            ensure_newline(out);
            out.push_str(&pad);
            out.push_str("[attachment]\n");
        }
        "blockquote" | "panel" | "expand" | "nestedExpand" | "listItem" | "tableRow" | "tableCell"
        | "tableHeader" | "doc" => {
            blocks(n.get("content"), indent, out, depth);
        }
        _ => {
            // Unknown node: its children's text (inline or block alike).
            if n.get("content").is_some() {
                let has_block = children(n).iter().any(is_block);
                if has_block {
                    blocks(n.get("content"), indent, out, depth);
                } else {
                    let t = inline_all(children(n), depth);
                    if !t.is_empty() {
                        ensure_newline(out);
                        out.push_str(&pad);
                        out.push_str(&t);
                        out.push('\n');
                    }
                }
            } else {
                let t = inline(n, depth);
                if !t.is_empty() {
                    ensure_newline(out);
                    out.push_str(&pad);
                    out.push_str(&t);
                    out.push('\n');
                }
            }
        }
    }
}

fn is_block(n: &Value) -> bool {
    matches!(
        kind(n),
        "paragraph"
            | "heading"
            | "bulletList"
            | "orderedList"
            | "codeBlock"
            | "table"
            | "blockquote"
            | "panel"
            | "mediaSingle"
            | "mediaGroup"
            | "rule"
    )
}

fn list_item(item: &Value, indent: usize, number: Option<u64>, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let marker = number.map_or_else(|| "- ".to_owned(), |n| format!("{n}. "));
    let mut first = true;
    for child in children(item) {
        match kind(child) {
            "bulletList" | "orderedList" => block(child, indent + 2, out, depth + 1),
            "paragraph" | "heading" => {
                ensure_newline(out);
                let lead = if first {
                    format!("{}{marker}", " ".repeat(indent))
                } else {
                    " ".repeat(indent + 2)
                };
                out.push_str(&lead);
                out.push_str(&inline_all(children(child), depth));
                out.push('\n');
                first = false;
            }
            _ => {
                if first {
                    ensure_newline(out);
                    out.push_str(&format!("{}{marker}\n", " ".repeat(indent)));
                    first = false;
                }
                block(child, indent + 2, out, depth + 1);
            }
        }
    }
    if first {
        ensure_newline(out);
        out.push_str(&format!("{}{}\n", " ".repeat(indent), marker.trim_end()));
    }
}

fn inline_all(nodes: &[Value], depth: usize) -> String {
    nodes.iter().map(|n| inline(n, depth)).collect()
}

fn inline(n: &Value, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return String::new();
    }
    match kind(n) {
        "text" => {
            let text = n.get("text").and_then(Value::as_str).unwrap_or("");
            let link = n
                .get("marks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|m| kind(m) == "link")
                .and_then(|m| m.pointer("/attrs/href"))
                .and_then(Value::as_str);
            match link {
                Some(href) if href != text => format!("{text} ({href})"),
                _ => text.to_owned(),
            }
        }
        "hardBreak" => "\n".to_owned(),
        "mention" => {
            let name = n.pointer("/attrs/text").and_then(Value::as_str).unwrap_or("");
            let name = name.trim_start_matches('@');
            if name.is_empty() {
                "@someone".to_owned()
            } else {
                format!("@{name}")
            }
        }
        "emoji" => n
            .pointer("/attrs/text")
            .or_else(|| n.pointer("/attrs/shortName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        "inlineCard" => n
            .pointer("/attrs/url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        "media" | "mediaInline" => "[attachment]".to_owned(),
        _ => inline_all(children(n), depth + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn doc(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }
    fn p(text: &str) -> Value {
        json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn paragraphs_and_headings() {
        let d = doc(json!([
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Title"}]},
            p("one"),
            p("two")
        ]));
        assert_eq!(adf_to_text(&d), "Title\none\ntwo");
    }

    #[test]
    fn nested_lists() {
        let d = doc(json!([
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [p("a"), {"type": "orderedList", "content": [
                    {"type": "listItem", "content": [p("a1")]},
                    {"type": "listItem", "content": [p("a2")]}
                ]}]},
                {"type": "listItem", "content": [p("b")]}
            ]}
        ]));
        assert_eq!(adf_to_text(&d), "- a\n  1. a1\n  2. a2\n- b");
    }

    #[test]
    fn code_blocks() {
        let d = doc(json!([
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn x() {}\nlet y;"}]},
            {"type": "codeBlock", "content": [{"type": "text", "text": "plain"}]}
        ]));
        assert_eq!(
            adf_to_text(&d),
            "```rust\nfn x() {}\nlet y;\n```\n```\nplain\n```"
        );
    }

    #[test]
    fn links_mentions_breaks() {
        let d = doc(json!([{"type": "paragraph", "content": [
            {"type": "text", "text": "see "},
            {"type": "text", "text": "docs", "marks": [{"type": "link", "attrs": {"href": "https://x.test/d"}}]},
            {"type": "hardBreak"},
            {"type": "mention", "attrs": {"id": "123", "text": "@Ada"}},
            {"type": "text", "text": " hi", "marks": [{"type": "strong"}]}
        ]}]));
        assert_eq!(adf_to_text(&d), "see docs (https://x.test/d)\n@Ada hi");
    }

    #[test]
    fn tables_and_media() {
        let cell = |t: &str| json!({"type": "tableCell", "content": [p(t)]});
        let d = doc(json!([
            {"type": "table", "content": [
                {"type": "tableRow", "content": [cell("h1"), cell("h2")]},
                {"type": "tableRow", "content": [cell("a"), cell("b")]}
            ]},
            {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"id": "x", "type": "file"}}]}
        ]));
        assert_eq!(adf_to_text(&d), "h1 | h2\na | b\n[attachment]");
    }

    #[test]
    fn unknown_and_non_document() {
        let d = doc(json!([
            {"type": "weirdBlock", "content": [{"type": "text", "text": "inside"}]},
            {"type": "blockquote", "content": [p("quoted")]}
        ]));
        assert_eq!(adf_to_text(&d), "inside\nquoted");
        for v in [
            json!(null),
            json!("x"),
            json!(3),
            json!([]),
            json!({"type": "paragraph"}),
            json!({}),
        ] {
            assert_eq!(adf_to_text(&v), "");
        }
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let mut v = json!({"type": "text", "text": "x"});
        for _ in 0..300 {
            v = json!({"type": "weird", "content": [v]});
        }
        let _ = adf_to_text(&doc(json!([v])));
    }

    fn arb_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            ".{0,10}".prop_map(Value::from),
            prop_oneof![
                Just("doc"),
                Just("paragraph"),
                Just("text"),
                Just("bulletList"),
                Just("orderedList"),
                Just("listItem"),
                Just("table"),
                Just("tableRow"),
                Just("codeBlock"),
                Just("mention"),
                Just("media"),
                Just("hardBreak"),
                Just("heading")
            ]
            .prop_map(|t| json!({"type": t})),
        ];
        leaf.prop_recursive(6, 64, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::from),
                (
                    prop_oneof![
                        Just("doc"), Just("paragraph"), Just("bulletList"), Just("orderedList"), Just("listItem"),
                        Just("table"), Just("tableRow"), Just("codeBlock"), Just("blockquote"), Just("x")
                    ],
                    proptest::collection::vec(inner.clone(), 0..5),
                    inner.clone(),
                    ".{0,8}",
                )
                    .prop_map(|(t, c, attrs, text)| json!({"type": t, "content": c, "attrs": attrs, "text": text, "marks": [attrs]})),
            ]
        })
    }

    proptest! {
        #[test]
        fn never_panics(v in arb_json()) {
            let _ = adf_to_text(&v);
            let _ = adf_to_text(&doc(v));
        }
    }
}
