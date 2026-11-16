//! Plan 0012: `corvus setup claude` through the real binary, against settings files in a temp `HOME`.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Output, Stdio};

use common::{Home, json_stdout, stderr};
use serde_json::Value;

fn settings(home: &Home) -> PathBuf {
    home.path().join(".claude").join("settings.json")
}

fn backup(home: &Home) -> PathBuf {
    home.path().join(".claude").join("settings.json.corvus-backup")
}

fn setup(home: &Home, args: &[&str], stdin: &str) -> Output {
    let mut child = home
        .corvus()
        .args(["setup", "claude"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn program() -> String {
    assert_cmd::cargo::cargo_bin("corvus")
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn creates_the_file_then_is_idempotent_then_removes() {
    let home = Home::new();
    let out = setup(&home, &["--yes"], "");
    let v = json_stdout(&out);
    assert_eq!(v["changed"], true);
    assert_eq!(v["backup"], Value::Null, "no file before, so no backup");
    assert!(!backup(&home).exists());
    let err = stderr(&out);
    assert!(err.contains("Adding the Corvus hooks"), "{err}");
    assert!(err.contains("\"mcp__corvus\""), "{err}");

    let s = read_json(&settings(&home));
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
    ] {
        assert_eq!(
            s["hooks"][event][0]["hooks"][0]["command"],
            format!("{} hook", program())
        );
    }
    for event in ["SessionStart", "Stop"] {
        let wait = &s["hooks"][event][0]["hooks"][1];
        assert_eq!(wait["command"], format!("{} hook wait", program()));
        assert_eq!(wait["asyncRewake"], true);
        assert_eq!(wait["timeout"], 86_400);
    }
    assert_eq!(s["hooks"]["PostToolUse"][0]["matcher"], "*");

    // Second run: says so, writes nothing.
    let before = std::fs::read(settings(&home)).unwrap();
    let out = setup(&home, &["--yes"], "");
    assert_eq!(json_stdout(&out)["changed"], false);
    assert!(stderr(&out).contains("Already set up"));
    assert_eq!(std::fs::read(settings(&home)).unwrap(), before);
    assert!(!backup(&home).exists());

    // Remove: back to an empty object, with the previous file as the backup.
    let out = setup(&home, &["--remove", "--yes"], "");
    let v = json_stdout(&out);
    assert_eq!(v["changed"], true);
    assert_eq!(v["removed"], true);
    assert_eq!(std::fs::read_to_string(settings(&home)).unwrap(), "{}\n");
    assert_eq!(std::fs::read(backup(&home)).unwrap(), before);
    let out = setup(&home, &["--remove"], "");
    assert_eq!(json_stdout(&out)["changed"], false);
    assert!(stderr(&out).contains("Nothing to remove"));
    // No temp file is left behind.
    assert!(!home.path().join(".claude/settings.json.corvus-tmp").exists());
}

#[test]
fn asks_first_and_keeps_everything_else() {
    let home = Home::new();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    let original = r#"{
  "model": "opus",
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "say done"
          }
        ]
      }
    ]
  },
  "permissions": {
    "allow": [
      "Bash(ls:*)"
    ]
  }
}
"#;
    std::fs::write(settings(&home), original).unwrap();

    // No answer, "n", and anything else all decline.
    for answer in ["", "n\n", "maybe\n"] {
        let out = setup(&home, &[], answer);
        let v = json_stdout(&out);
        assert_eq!(v["changed"], false, "{answer:?}");
        assert_eq!(v["declined"], true, "{answer:?}");
        let err = stderr(&out);
        assert!(err.contains("Write this change? [y/N]"), "{err}");
        assert!(err.contains("+ ") && err.contains("hook wait"), "{err}");
        assert!(
            !err.contains("\"model\""),
            "unchanged lines far from the change are not shown: {err}"
        );
        assert_eq!(std::fs::read_to_string(settings(&home)).unwrap(), original);
        assert!(!backup(&home).exists());
    }

    let out = setup(&home, &[], "y\n");
    assert_eq!(json_stdout(&out)["changed"], true);
    assert_eq!(std::fs::read_to_string(backup(&home)).unwrap(), original);
    let text = std::fs::read_to_string(settings(&home)).unwrap();
    let s = read_json(&settings(&home));
    assert_eq!(s["model"], "opus");
    assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)");
    assert_eq!(s["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
    assert_eq!(
        s["hooks"]["Stop"][1]["hooks"][0]["command"],
        format!("{} hook", program())
    );
    assert!(text.find("\"model\"").unwrap() < text.find("\"hooks\"").unwrap());
    assert!(text.find("\"hooks\"").unwrap() < text.find("\"permissions\"").unwrap());

    // Removing gives the original back byte for byte.
    assert_eq!(
        json_stdout(&setup(&home, &["--remove", "--yes"], ""))["changed"],
        true
    );
    assert_eq!(std::fs::read_to_string(settings(&home)).unwrap(), original);
}

#[test]
fn refuses_a_file_that_is_not_json() {
    let home = Home::new();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    let broken = "{ \"hooks\": { // half-edited\n";
    std::fs::write(settings(&home), broken).unwrap();
    for args in [&["--yes"][..], &["--remove", "--yes"][..]] {
        let out = setup(&home, args, "");
        assert_eq!(out.status.code(), Some(2));
        let err = stderr(&out);
        assert!(
            err.contains("\"validation\"") && err.contains("not valid JSON"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(settings(&home)).unwrap(), broken);
        assert!(!backup(&home).exists());
    }
}

#[test]
fn honours_claude_config_dir_and_file() {
    let home = Home::new();
    let dir = home.path().join("elsewhere");
    let out = home
        .corvus()
        .env("CLAUDE_CONFIG_DIR", &dir)
        .args(["setup", "claude", "--yes"])
        .output()
        .unwrap();
    assert_eq!(json_stdout(&out)["changed"], true);
    assert!(dir.join("settings.json").exists());
    assert!(!settings(&home).exists());

    let project = home.path().join("proj/.claude/settings.json");
    let out = setup(&home, &["--yes", "--file", project.to_str().unwrap()], "");
    assert_eq!(json_stdout(&out)["changed"], true);
    assert!(read_json(&project)["hooks"]["Stop"].is_array());
    assert!(!settings(&home).exists());
}

/// The paste-it-yourself snippet in the docs is exactly what the command writes (with `corvus` on `PATH`).
#[test]
fn docs_snippet_matches_the_command() {
    let docs = include_str!("../../../docs/delivery-helpers.md");
    let snippet = corvus_core::delivery::claude_setup::snippet("corvus");
    assert!(
        docs.contains(&format!("```json\n{snippet}```")),
        "docs/delivery-helpers.md drifted from `corvus setup claude`; it should contain:\n{snippet}"
    );
    assert!(docs.contains(corvus_core::delivery::claude_setup::ALLOW_RULE));
}

#[cfg(unix)]
#[test]
fn writes_through_a_symlink_and_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let real_dir = home.path().join("dotfiles");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    let real = real_dir.join("claude-settings.json");
    std::fs::write(&real, "{}\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&real, settings(&home)).unwrap();

    assert_eq!(json_stdout(&setup(&home, &["--yes"], ""))["changed"], true);
    assert!(
        std::fs::symlink_metadata(settings(&home))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read_json(&real)["hooks"]["Stop"].is_array());
    assert_eq!(
        std::fs::metadata(&real).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::read_to_string(real_dir.join("claude-settings.json.corvus-backup")).unwrap(),
        "{}\n"
    );
}
