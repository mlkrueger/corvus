//! Keeps `docs/usage.md` honest: every HTTP route, CLI command, MCP tool and `CORVUS_*` variable the code
//! defines must appear in the document.

use std::collections::BTreeSet;
use std::process::Command;

const USAGE: &str = include_str!("../../../docs/usage.md");
const MCP_SOURCE: &str = include_str!("../../corvus-mcp/src/lib.rs");
const CONFIG_SOURCE: &str = include_str!("../../corvus-core/src/config.rs");

fn missing(wanted: impl IntoIterator<Item = String>) -> Vec<String> {
    wanted
        .into_iter()
        .filter(|w| !USAGE.contains(w.as_str()))
        .collect()
}

#[test]
fn every_http_route_is_documented() {
    let table = corvus_http::route_table();
    assert!(table.len() >= 40, "route table looks truncated: {}", table.len());
    let absent = missing(table.iter().map(|(m, p)| format!("`{m} {p}`")));
    assert!(absent.is_empty(), "routes missing from docs/usage.md: {absent:?}");
}

#[test]
fn every_top_level_cli_command_is_documented() {
    let out = Command::new(assert_cmd::cargo::cargo_bin("corvus"))
        .arg("--help")
        .output()
        .expect("run corvus --help");
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).expect("utf8 help");
    let names: Vec<String> = help
        .lines()
        .skip_while(|l| l.trim() != "Commands:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| *n != "help")
        .map(str::to_owned)
        .collect();
    assert!(
        names.len() >= 20,
        "could not parse the Commands: section: {names:?}"
    );
    let absent = missing(names.iter().map(|n| format!("corvus {n}")));
    assert!(
        absent.is_empty(),
        "CLI commands missing from docs/usage.md: {absent:?}"
    );
}

/// Tool names, read from the `tool::<T>("name", ...)` registrations in `tool_defs`.
fn mcp_tool_names() -> Vec<String> {
    let defs = MCP_SOURCE
        .split("fn tool_defs()")
        .nth(1)
        .expect("tool_defs in corvus-mcp");
    let defs = defs.split("fn error_json").next().expect("end of tool_defs");
    let mut names = Vec::new();
    let mut rest = defs;
    while let Some(i) = rest.find("tool::<") {
        rest = &rest[i + "tool::<".len()..];
        let after_paren = &rest[rest.find('(').expect("open paren")..];
        let start = after_paren.find('"').expect("tool name literal") + 1;
        let end = start + after_paren[start..].find('"').expect("closing quote");
        names.push(after_paren[start..end].to_owned());
    }
    names
}

#[test]
fn every_mcp_tool_is_documented() {
    let names = mcp_tool_names();
    assert_eq!(names.len(), 20, "expected twenty MCP tools, found {names:?}");
    let absent = missing(names.iter().map(|n| format!("`{n}`")));
    assert!(
        absent.is_empty(),
        "MCP tools missing from docs/usage.md: {absent:?}"
    );
}

#[test]
fn every_config_variable_is_documented() {
    let code = CONFIG_SOURCE.split("#[cfg(test)]").next().expect("config source");
    let mut vars = BTreeSet::new();
    let mut rest = code;
    while let Some(i) = rest.find("CORVUS_") {
        rest = &rest[i..];
        let end = rest
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        let name = rest[..end].trim_end_matches('_');
        if name.len() > "CORVUS".len() {
            vars.insert(name.to_owned());
        }
        rest = &rest[end..];
    }
    assert!(vars.len() >= 20, "variable scan looks wrong: {vars:?}");
    let absent = missing(vars.into_iter().map(|v| format!("`{v}")));
    assert!(
        absent.is_empty(),
        "CORVUS_* variables missing from docs/usage.md: {absent:?}"
    );
}
