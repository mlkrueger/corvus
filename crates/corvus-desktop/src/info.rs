//! `app_info` and `install_cli`: finding the `corvus` binary, the "Connect an agent" snippets, and linking the
//! binary into `~/.local/bin` (never with elevated rights).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Adapter {
    pub name: String,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub corvus_path: Option<String>,
    pub cli_installed: bool,
    pub adapters: Vec<Adapter>,
    pub mcp_snippet: String,
    pub hook_snippet: String,
}

fn exe_name(base: &str) -> String {
    format!("{base}{}", std::env::consts::EXE_SUFFIX)
}

fn is_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

/// Finds `name` in the directories of a `PATH`-style value.
pub fn find_on_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let file = exe_name(name);
    std::env::split_paths(path_var)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(&file))
        .find(|p| is_file(p))
}

/// The `corvus` binary: next to the app executable (the bundled one), then on `PATH`.
pub fn find_corvus(exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(dir) = exe_dir {
        let sibling = dir.join(exe_name("corvus"));
        if is_file(&sibling) {
            return Some(sibling);
        }
    }
    find_on_path("corvus", path_var)
}

pub fn cli_link_path(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join(exe_name("corvus"))
}

pub fn mcp_snippet(corvus: &str) -> String {
    let v = json!({ "mcpServers": { "corvus": { "command": corvus, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

pub fn hook_snippet(corvus: &str) -> String {
    // Claude Code hook contract (system/06): UserPromptSubmit, PostToolUse and Stop. Settings `env` is not
    // documented to reach hooks, so the agent name is set in the command itself; replace YOUR-AGENT-NAME.
    let command = format!("CORVUS_AGENT_NAME=YOUR-AGENT-NAME {corvus} hook");
    let hook = json!([{ "type": "command", "command": command }]);
    let v = json!({ "hooks": {
        "UserPromptSubmit": [{ "hooks": hook.clone() }],
        "PostToolUse": [{ "matcher": "*", "hooks": hook.clone() }],
        "Stop": [{ "hooks": hook }],
    } });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Everything `app_info` reports, from explicit inputs (tests pass temp directories).
pub fn gather(exe_dir: Option<&Path>, path_var: Option<&OsStr>, home: Option<&Path>) -> AppInfo {
    let corvus = find_corvus(exe_dir, path_var);
    let command = corvus
        .as_ref()
        .map_or_else(|| "corvus".to_owned(), |p| p.to_string_lossy().into_owned());
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        corvus_path: corvus.as_ref().map(|p| p.to_string_lossy().into_owned()),
        cli_installed: home.is_some_and(|h| cli_link_path(h).exists()),
        adapters: [("claude-code", "claude"), ("codex", "codex")]
            .into_iter()
            .map(|(name, bin)| Adapter {
                name: name.to_owned(),
                installed: find_on_path(bin, path_var).is_some(),
            })
            .collect(),
        mcp_snippet: mcp_snippet(&command),
        hook_snippet: hook_snippet(&command),
    }
}

/// `gather` for the running process.
pub fn gather_from_env() -> AppInfo {
    let exe = std::env::current_exe().ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    gather(
        exe.as_deref().and_then(Path::parent),
        std::env::var_os("PATH").as_deref(),
        home.as_deref(),
    )
}

/// Symlinks `source` to `<home>/.local/bin/corvus`. Replaces an existing symlink; refuses anything else.
#[cfg(unix)]
pub fn install_cli(home: &Path, source: &Path) -> Result<String, String> {
    use std::os::unix::fs::symlink;

    let source = source
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", source.display()))?;
    let link = cli_link_path(home);
    let dir = link.parent().ok_or("invalid link location")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    match std::fs::symlink_metadata(&link) {
        Ok(m) if m.file_type().is_symlink() => {
            std::fs::remove_file(&link).map_err(|e| format!("cannot replace {}: {e}", link.display()))?;
        }
        Ok(_) => {
            return Err(format!(
                "{} already exists and is not a symlink; remove it yourself if you want to replace it",
                link.display()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot inspect {}: {e}", link.display())),
    }
    symlink(&source, &link).map_err(|e| format!("cannot link {}: {e}", link.display()))?;
    Ok(format!("Linked {} to {}", link.display(), source.display()))
}

#[cfg(not(unix))]
pub fn install_cli(_home: &Path, _source: &Path) -> Result<String, String> {
    Err("installing the command-line tool is supported on macOS and Linux only".into())
}

/// `install_cli` for the running process: finds the binary, then links it.
pub fn install_cli_from_env() -> Result<String, String> {
    let info = gather_from_env();
    let source = info
        .corvus_path
        .ok_or("the corvus binary was not found next to the app or on PATH")?;
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    install_cli(Path::new(&home), Path::new(&source))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_bin(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        p
    }

    #[test]
    fn install_cli_creates_symlink_and_is_repeatable() {
        let t = tempfile::tempdir().expect("tmp");
        let src = fake_bin(t.path(), "corvus");
        let home = t.path().join("home");
        std::fs::create_dir(&home).expect("home");
        let msg = install_cli(&home, &src).expect("install");
        assert!(msg.contains(".local/bin/corvus"), "{msg}");
        let link = home.join(".local/bin/corvus");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(&link).expect("link"),
            src.canonicalize().expect("canon")
        );
        install_cli(&home, &src).expect("a second run replaces the symlink");
    }

    #[test]
    fn install_cli_refuses_to_clobber_a_regular_file() {
        let t = tempfile::tempdir().expect("tmp");
        let src = fake_bin(t.path(), "corvus");
        let home = t.path().join("home");
        std::fs::create_dir_all(home.join(".local/bin")).expect("dirs");
        let existing = home.join(".local/bin/corvus");
        std::fs::write(&existing, "mine").expect("write");
        let err = install_cli(&home, &src).expect_err("must refuse");
        assert!(err.contains("not a symlink"), "{err}");
        assert_eq!(std::fs::read_to_string(&existing).expect("read"), "mine");
    }

    #[test]
    fn install_cli_fails_for_a_missing_source() {
        let t = tempfile::tempdir().expect("tmp");
        assert!(install_cli(t.path(), &t.path().join("nope")).is_err());
    }

    #[test]
    fn finds_sibling_before_path_and_reports_adapters() {
        let t = tempfile::tempdir().expect("tmp");
        let exe_dir = t.path().join("app");
        let path_dir = t.path().join("bin");
        std::fs::create_dir_all(&exe_dir).expect("d");
        std::fs::create_dir_all(&path_dir).expect("d");
        let sibling = fake_bin(&exe_dir, "corvus");
        fake_bin(&path_dir, "corvus");
        fake_bin(&path_dir, "claude");
        let path = std::env::join_paths([&path_dir]).expect("join");
        let home = t.path().join("home");

        let info = gather(Some(&exe_dir), Some(&path), Some(&home));
        assert_eq!(info.corvus_path.as_deref(), Some(sibling.to_str().expect("utf8")));
        assert!(!info.cli_installed);
        assert_eq!(
            info.adapters,
            vec![
                Adapter {
                    name: "claude-code".into(),
                    installed: true
                },
                Adapter {
                    name: "codex".into(),
                    installed: false
                },
            ]
        );
        let mcp: serde_json::Value = serde_json::from_str(&info.mcp_snippet).expect("json");
        assert_eq!(
            mcp["mcpServers"]["corvus"]["command"],
            sibling.to_str().expect("utf8")
        );
        assert_eq!(mcp["mcpServers"]["corvus"]["args"], json!(["mcp"]));
        assert!(info.hook_snippet.contains(" hook"));
        for event in ["UserPromptSubmit", "PostToolUse", "Stop"] {
            assert!(info.hook_snippet.contains(event), "{event}");
        }
        assert!(info.hook_snippet.contains("CORVUS_AGENT_NAME="));

        let info = gather(None, Some(&path), Some(&home));
        assert_eq!(info.corvus_path.as_deref(), path_dir.join("corvus").to_str());
        install_cli(&home, Path::new(info.corvus_path.as_deref().expect("found"))).expect("install");
        assert!(gather(None, Some(&path), Some(&home)).cli_installed);
    }

    #[test]
    fn nothing_found_falls_back_to_the_bare_command() {
        let info = gather(None, None, None);
        assert_eq!(info.corvus_path, None);
        assert!(info.mcp_snippet.contains("\"command\": \"corvus\""));
        assert!(info.adapters.iter().all(|a| !a.installed));
    }
}
