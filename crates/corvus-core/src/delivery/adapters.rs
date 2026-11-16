//! Loading agent-tool adapters ([system/06](../../../../spec/system/06-delivery-helpers.md#agent-adapters), D0021).
//!
//! Built-ins are embedded in the binary. User adapters live in `$STATE/adapters/<name>.toml` and override a
//! built-in of the same name. A user file is trusted only if it is a regular file owned by the same user as the
//! state directory and not group- or world-writable. The pure model and validation are in
//! `corvus_domain::wake`; this module reads files and runs the `check` command.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use corvus_domain::wake::validate_adapter_name;
use corvus_domain::{Adapter, BindingPattern, CorvusError, RawAdapter, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::os::unix::process::CommandExt;

/// Built-in adapters: (name, TOML text).
const BUILTINS: &[(&str, &str)] = &[
    ("codex", include_str!("../../adapters/codex.toml")),
    ("opencode", include_str!("../../adapters/opencode.toml")),
];

/// How long a `check` command may run.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Builtin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedAdapter {
    pub adapter: Adapter,
    pub source: Source,
}

pub fn adapters_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("adapters")
}

// The shape of an adapter file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileAdapter {
    name: String,
    title: Option<String>,
    #[serde(default)]
    binding: BTreeMap<String, FileBinding>,
    wake: FileWake,
    check: Option<FileCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileBinding {
    #[serde(default)]
    required: bool,
    pattern: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWake {
    command: Vec<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileCheck {
    command: Vec<String>,
}

/// Parses and validates adapter TOML. `label` names the source in errors; the file's `name` must equal
/// `expected_name`.
pub fn parse(text: &str, label: &str, expected_name: &str) -> Result<Adapter> {
    let bad = |m: String| CorvusError::Validation(format!("{label}: {m}"));
    let f: FileAdapter = toml::from_str(text).map_err(|e| bad(e.message().trim().to_owned()))?;
    if f.name != expected_name {
        return Err(bad(format!(
            "name {:?} must equal the file name {expected_name:?}",
            f.name
        )));
    }
    Adapter::validate(RawAdapter {
        name: f.name,
        title: f.title,
        binding: f
            .binding
            .into_iter()
            .map(|(k, b)| (k, (b.required, b.pattern)))
            .collect(),
        wake: f.wake.command,
        timeout_seconds: f.wake.timeout_seconds,
        check: f.check.map(|c| c.command),
    })
    .map_err(|e| bad(e.to_string()))
}

fn builtin(name: &str) -> Option<Result<Adapter>> {
    BUILTINS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, text)| parse(text, &format!("built-in adapter {n}"), n))
}

/// Why a user adapter file must not be loaded, if it must not. `state_uid` is the owner of the state directory.
fn untrusted(meta: &std::fs::Metadata, state_uid: u32) -> Option<String> {
    if !meta.is_file() {
        return Some("not a regular file (symlinks are not followed)".into());
    }
    if meta.uid() != state_uid {
        return Some(format!(
            "owned by uid {} but the state directory is owned by uid {state_uid}",
            meta.uid()
        ));
    }
    if meta.mode() & 0o022 != 0 {
        return Some(format!(
            "mode {:o} is group- or world-writable; run chmod go-w on it",
            meta.mode() & 0o777
        ));
    }
    None
}

fn load_user(state_dir: &Path, name: &str) -> Result<Option<Adapter>> {
    let path = adapters_dir(state_dir).join(format!("{name}.toml"));
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CorvusError::Internal(format!("{}: {e}", path.display()))),
    };
    let state_uid = std::fs::metadata(state_dir)
        .map_err(|e| CorvusError::Internal(format!("{}: {e}", state_dir.display())))?
        .uid();
    if let Some(why) = untrusted(&meta, state_uid) {
        return Err(CorvusError::Validation(format!(
            "{}: refusing to load adapter: {why}",
            path.display()
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CorvusError::Internal(format!("{}: {e}", path.display())))?;
    parse(&text, &path.display().to_string(), name).map(Some)
}

/// Loads adapter `name`: the user's file if there is one (an invalid user file is an error, never a silent
/// fall-back to the built-in), else the built-in.
pub fn load(state_dir: &Path, name: &str) -> Result<LoadedAdapter> {
    validate_adapter_name(name)?; // the name becomes a file name
    if let Some(adapter) = load_user(state_dir, name)? {
        return Ok(LoadedAdapter {
            adapter,
            source: Source::User,
        });
    }
    match builtin(name) {
        Some(adapter) => Ok(LoadedAdapter {
            adapter: adapter?,
            source: Source::Builtin,
        }),
        None => Err(CorvusError::NotFound(format!(
            "no adapter named {name:?} (looked for {} and the built-ins)",
            adapters_dir(state_dir).join(format!("{name}.toml")).display()
        ))),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AdapterEntry {
    pub name: String,
    pub title: Option<String>,
    pub source: Source,
    pub valid: bool,
    pub error: Option<String>,
}

/// Every built-in and every `*.toml` in the adapters directory, with whether it loads.
pub fn list(state_dir: &Path) -> Vec<AdapterEntry> {
    let mut names: BTreeSet<String> = BUILTINS.iter().map(|(n, _)| (*n).to_owned()).collect();
    if let Ok(rd) = std::fs::read_dir(adapters_dir(state_dir)) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("toml")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            {
                names.insert(stem.to_owned());
            }
        }
    }
    names
        .into_iter()
        .map(|name| match load(state_dir, &name) {
            Ok(l) => AdapterEntry {
                title: Some(l.adapter.title.clone()),
                source: l.source,
                valid: true,
                error: None,
                name,
            },
            Err(e) => AdapterEntry {
                source: if load_user_exists(state_dir, &name) {
                    Source::User
                } else {
                    Source::Builtin
                },
                title: None,
                valid: false,
                error: Some(e.to_string()),
                name,
            },
        })
        .collect()
}

fn load_user_exists(state_dir: &Path, name: &str) -> bool {
    std::fs::symlink_metadata(adapters_dir(state_dir).join(format!("{name}.toml"))).is_ok()
}

/// The adapter as JSON, for `adapter show`.
pub fn to_json(l: &LoadedAdapter) -> Value {
    let a = &l.adapter;
    let binding: BTreeMap<_, _> = a
        .binding
        .iter()
        .map(|(k, (required, pattern))| {
            (
                k.clone(),
                json!({ "required": required, "pattern": pattern_text(pattern) }),
            )
        })
        .collect();
    json!({
        "name": a.name,
        "title": a.title,
        "source": l.source,
        "binding": binding,
        "wake": { "command": a.wake, "timeout_seconds": a.timeout_seconds },
        "check": a.check.as_ref().map(|c| json!({ "command": c })),
    })
}

fn pattern_text(p: &BindingPattern) -> String {
    match p {
        BindingPattern::Uuid => "uuid".to_owned(),
        BindingPattern::Charset { allowed, min, max } => format!("chars:{allowed}:{min}-{max}"),
    }
}

/// Runs the adapter's `check` command (argument vector, [`CHECK_TIMEOUT`]).
///
/// `status` is `ok`, `not_installed` (the program was not found), `check_failed` (it ran and failed or timed
/// out) or `no_check` (the adapter has no check command). `installed` is true/false, or null for `no_check`.
pub fn check(adapter: &Adapter, timeout: Duration) -> Value {
    let Some(argv) = &adapter.check else {
        return json!({ "adapter": adapter.name, "installed": null, "status": "no_check",
                       "version_output": null, "detail": "this adapter has no [check] command" });
    };
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return json!({ "adapter": adapter.name, "installed": false, "status": "not_installed",
                           "version_output": null, "detail": format!("{:?} was not found", argv[0]) });
        }
        Err(e) => {
            return json!({ "adapter": adapter.name, "installed": true, "status": "check_failed",
                           "version_output": null, "detail": format!("could not run {:?}: {e}", argv[0]) });
        }
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if text.is_empty() {
                text = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            }
            let text: String = text.chars().take(OUTPUT_CHARS).collect();
            if out.status.success() {
                json!({ "adapter": adapter.name, "installed": true, "status": "ok",
                        "version_output": text, "detail": null })
            } else {
                json!({ "adapter": adapter.name, "installed": true, "status": "check_failed",
                        "version_output": text, "detail": format!("check command {}", out.status) })
            }
        }
        Ok(Err(e)) => json!({ "adapter": adapter.name, "installed": true, "status": "check_failed",
                              "version_output": null, "detail": e.to_string() }),
        Err(_) => {
            kill_group(pid);
            json!({ "adapter": adapter.name, "installed": true, "status": "check_failed",
                    "version_output": null, "detail": format!("check command timed out after {}s", timeout.as_secs()) })
        }
    }
}

/// SIGKILLs the process group led by `pgid` (children started with `process_group(0)` lead their own group).
/// `unsafe` is denied workspace-wide, so this runs `kill` with an argument vector instead of calling libc.
pub(crate) fn kill_group(pgid: u32) {
    let r = Command::new("kill")
        .args(["-s", "KILL", "--"])
        .arg(format!("-{pgid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !r.is_ok_and(|s| s.success()) {
        tracing::warn!(pgid, "kill of process group failed (already gone?)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn builtins_are_valid() {
        for (name, text) in BUILTINS {
            let a = parse(text, name, name).unwrap();
            assert_eq!(&a.name, name);
        }
    }

    #[test]
    fn wrong_owner_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.toml");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::symlink_metadata(&p).unwrap();
        assert!(untrusted(&meta, meta.uid()).is_none());
        let why = untrusted(&meta, meta.uid().wrapping_add(1)).unwrap();
        assert!(why.contains("owned by uid"), "{why}");
    }

    #[test]
    fn group_writable_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.toml");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o660)).unwrap();
        let meta = std::fs::symlink_metadata(&p).unwrap();
        assert!(untrusted(&meta, meta.uid()).unwrap().contains("writable"));
    }
}
