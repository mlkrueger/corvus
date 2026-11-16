//! Plan 0007 verification: adapter loading, `check`, and bindings.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvus_core::delivery::adapters::{self, Source};
use corvus_core::delivery::bindings;
use corvus_domain::CorvusError;

const UUID: &str = "01890000-0000-7000-8000-000000000000";

struct State {
    dir: tempfile::TempDir,
}

fn state() -> State {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("adapters")).unwrap();
    State { dir }
}

impl State {
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn write(&self, file: &str, text: &str) -> PathBuf {
        let p = self.path().join("adapters").join(file);
        std::fs::write(&p, text).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        p
    }
}

fn msg(r: Result<impl std::fmt::Debug, CorvusError>) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn bad_adapters_are_rejected_at_load_with_clear_messages() {
    let s = state();
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "unknown placeholder",
            "name=\"t\"\n[wake]\ncommand=[\"tool\",\"{message}\"]\n",
            &["t.toml", "{message}", "not a placeholder"],
        ),
        (
            "embedded placeholder",
            "name=\"t\"\n[wake]\ncommand=[\"tool\",\"--note={note}\"]\n",
            &["t.toml", "--note={note}"],
        ),
        ("missing command", "name=\"t\"\n[wake]\n", &["t.toml", "command"]),
        (
            "empty command",
            "name=\"t\"\n[wake]\ncommand=[]\n",
            &["t.toml", "must not be empty"],
        ),
        (
            "name mismatch",
            "name=\"other\"\n[wake]\ncommand=[\"tool\"]\n",
            &["t.toml", "must equal the file name"],
        ),
        (
            "unknown field",
            "name=\"t\"\nbody=\"{body}\"\n[wake]\ncommand=[\"tool\"]\n",
            &["t.toml", "body"],
        ),
        ("not toml", "name = = =", &["t.toml"]),
    ];
    for (what, text, wants) in cases {
        s.write("t.toml", text);
        let m = msg(adapters::load(s.path(), "t"));
        for w in *wants {
            assert!(m.contains(w), "{what}: {m:?} should mention {w:?}");
        }
    }
}

#[test]
fn permissions_are_enforced() {
    let s = state();
    let p = s.write("t.toml", "name=\"t\"\n[wake]\ncommand=[\"tool\"]\n");
    assert!(adapters::load(s.path(), "t").is_ok());
    for mode in [0o660, 0o606, 0o666] {
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        let m = msg(adapters::load(s.path(), "t"));
        assert!(m.contains("t.toml") && m.contains("writable"), "{mode:o}: {m}");
    }
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        adapters::load(s.path(), "t").is_ok(),
        "readable by others is fine"
    );

    // Symlinks are not followed.
    let target = s.path().join("elsewhere.toml");
    std::fs::write(&target, "name=\"u\"\n[wake]\ncommand=[\"tool\"]\n").unwrap();
    std::os::unix::fs::symlink(&target, s.path().join("adapters/u.toml")).unwrap();
    assert!(msg(adapters::load(s.path(), "u")).contains("not a regular file"));
    // A name cannot escape the directory.
    assert!(adapters::load(s.path(), "../t").is_err());
    // Wrong-owner is covered by the unit test `wrong_owner_is_refused` in adapters.rs: a test cannot chown a file
    // to another user without privileges.
}

#[test]
fn a_user_adapter_overrides_a_builtin_and_a_broken_one_does_not_fall_back() {
    let s = state();
    let b = adapters::load(s.path(), "codex").unwrap();
    assert_eq!(b.source, Source::Builtin);
    assert_eq!(b.adapter.title, "OpenAI Codex CLI");
    assert_eq!(b.adapter.timeout_seconds, 30);

    s.write(
        "codex.toml",
        "name=\"codex\"\ntitle=\"My Codex\"\n[binding]\nconversation={required=false,pattern=\"uuid\"}\n[wake]\ncommand=[\"mycodex\",\"{note}\"]\n",
    );
    let u = adapters::load(s.path(), "codex").unwrap();
    assert_eq!(u.source, Source::User);
    assert_eq!(u.adapter.title, "My Codex");
    assert_eq!(u.adapter.wake, vec!["mycodex", "{note}"]);

    s.write(
        "codex.toml",
        "name=\"codex\"\n[wake]\ncommand=[\"x\",\"{nope}\"]\n",
    );
    assert!(
        msg(adapters::load(s.path(), "codex")).contains("{nope}"),
        "invalid user file is an error"
    );
}

#[test]
fn list_show_and_check_report_correctly() {
    let s = state();
    s.write(
        "mytool.toml",
        "name=\"mytool\"\ntitle=\"My Tool\"\n[wake]\ncommand=[\"mytool\",\"{note}\"]\n[check]\ncommand=[\"/nonexistent/mytool-xyz\",\"--version\"]\n",
    );
    s.write(
        "broken.toml",
        "name=\"broken\"\n[wake]\ncommand=[\"x\",\"{zzz}\"]\n",
    );
    let list = serde_json::to_value(adapters::list(s.path())).unwrap();
    let by = |n: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == n)
            .unwrap()
            .clone()
    };
    let codex = by("codex");
    assert_eq!(
        (codex["source"].as_str(), codex["valid"].as_bool()),
        (Some("builtin"), Some(true))
    );
    assert_eq!(codex["title"], "OpenAI Codex CLI");
    let my = by("mytool");
    assert_eq!(
        (my["source"].as_str(), my["valid"].as_bool()),
        (Some("user"), Some(true))
    );
    let broken = by("broken");
    assert_eq!(
        (broken["source"].as_str(), broken["valid"].as_bool()),
        (Some("user"), Some(false))
    );
    assert!(broken["error"].as_str().unwrap().contains("{zzz}"));

    let shown = adapters::to_json(&adapters::load(s.path(), "codex").unwrap());
    assert_eq!(shown["wake"]["command"][0], "codex");
    assert_eq!(shown["binding"]["conversation"]["pattern"], "uuid");
    assert_eq!(shown["binding"]["conversation"]["required"], true);

    // "not installed" is distinct from "check failed".
    let not_installed = adapters::check(
        &adapters::load(s.path(), "mytool").unwrap().adapter,
        Duration::from_secs(5),
    );
    assert_eq!(not_installed["installed"], false);
    assert_eq!(not_installed["status"], "not_installed");

    s.write(
        "good.toml",
        "name=\"good\"\n[wake]\ncommand=[\"x\"]\n[check]\ncommand=[\"/bin/sh\",\"-c\",\"echo tool 1.2.3\"]\n",
    );
    let ok = adapters::check(
        &adapters::load(s.path(), "good").unwrap().adapter,
        Duration::from_secs(5),
    );
    assert_eq!(
        (ok["installed"].as_bool(), ok["status"].as_str()),
        (Some(true), Some("ok"))
    );
    assert_eq!(ok["version_output"], "tool 1.2.3");

    s.write("bad.toml", "name=\"bad\"\n[wake]\ncommand=[\"x\"]\n[check]\ncommand=[\"/bin/sh\",\"-c\",\"echo boom >&2; exit 3\"]\n");
    let failed = adapters::check(
        &adapters::load(s.path(), "bad").unwrap().adapter,
        Duration::from_secs(5),
    );
    assert_eq!(
        (failed["installed"].as_bool(), failed["status"].as_str()),
        (Some(true), Some("check_failed"))
    );
    assert_eq!(failed["version_output"], "boom");

    s.write(
        "slow.toml",
        "name=\"slow\"\n[wake]\ncommand=[\"x\"]\n[check]\ncommand=[\"/bin/sleep\",\"30\"]\n",
    );
    let t = std::time::Instant::now();
    let slow = adapters::check(
        &adapters::load(s.path(), "slow").unwrap().adapter,
        Duration::from_millis(300),
    );
    assert_eq!(slow["status"], "check_failed");
    assert!(t.elapsed() < Duration::from_secs(5), "timed out promptly");

    s.write("nocheck.toml", "name=\"nocheck\"\n[wake]\ncommand=[\"x\"]\n");
    let n = adapters::check(
        &adapters::load(s.path(), "nocheck").unwrap().adapter,
        Duration::from_secs(1),
    );
    assert_eq!(n["status"], "no_check");

    assert!(matches!(
        adapters::load(s.path(), "nosuch"),
        Err(CorvusError::NotFound(_))
    ));
}

#[test]
fn bind_validates_names_and_patterns_allows_one_binding_and_force_rebinds() {
    let s = state();
    let codex = adapters::load(s.path(), "codex").unwrap().adapter;
    // agent-name grammar
    for bad in ["", "-x", "has space", "a/b", "../x"] {
        assert!(
            bindings::bind(s.path(), bad, &codex, Some(UUID), false).is_err(),
            "{bad:?}"
        );
    }
    // binding pattern and requiredness
    assert!(
        msg(bindings::bind(s.path(), "bob", &codex, Some("not-a-uuid"), false)).contains("does not match")
    );
    assert!(msg(bindings::bind(s.path(), "bob", &codex, None, false)).contains("needs --conversation"));
    assert!(
        bindings::list(s.path()).unwrap().is_empty(),
        "failed binds leave nothing behind"
    );

    let b = bindings::bind(s.path(), "bob", &codex, Some(UUID), false).unwrap();
    assert_eq!(
        (b.adapter.as_str(), b.conversation.as_deref()),
        ("codex", Some(UUID))
    );
    let other = "01890000-0000-7000-8000-000000000001";
    let err = bindings::bind(s.path(), "bob", &codex, Some(other), false).unwrap_err();
    assert!(matches!(err, CorvusError::Conflict(_)), "{err}");
    assert_eq!(
        bindings::get(s.path(), "bob")
            .unwrap()
            .unwrap()
            .conversation
            .as_deref(),
        Some(UUID)
    );
    bindings::bind(s.path(), "bob", &codex, Some(other), true).unwrap();
    assert_eq!(
        bindings::get(s.path(), "bob")
            .unwrap()
            .unwrap()
            .conversation
            .as_deref(),
        Some(other)
    );

    // File format and mode.
    let path = bindings::bindings_path(s.path());
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["bob"]["adapter"], "codex");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    bindings::unbind(s.path(), "bob").unwrap();
    assert!(bindings::list(s.path()).unwrap().is_empty());
    assert!(matches!(
        bindings::unbind(s.path(), "bob"),
        Err(CorvusError::NotFound(_))
    ));
}

#[test]
fn a_malformed_bindings_file_is_an_error_naming_the_file() {
    let s = state();
    std::fs::create_dir_all(s.path().join("agents")).unwrap();
    std::fs::write(s.path().join("agents/bindings.json"), "{nope").unwrap();
    assert!(msg(bindings::list(s.path())).contains("bindings.json"));
}
