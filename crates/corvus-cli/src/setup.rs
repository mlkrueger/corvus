//! `corvus setup claude`: add or remove the Corvus hooks in Claude Code's settings file
//! ([system/06](../../../spec/system/06-delivery-helpers.md)). The edit itself is planned by
//! `corvus_core::delivery::claude_setup`; this file reads, shows, asks and writes.

use corvus_core::delivery::claude_setup;
use corvus_domain::CorvusError;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand, Debug)]
pub enum SetupCommand {
    /// Add the Corvus hooks to Claude Code's user settings (~/.claude/settings.json): new mail is shown in
    /// every session and an idle session is woken by mail addressed to it. Shows the change and asks first.
    Claude {
        /// Take the Corvus hooks out again.
        #[arg(long)]
        remove: bool,
        /// Do not ask before writing.
        #[arg(long)]
        yes: bool,
        /// Another settings file, for example a project's `.claude/settings.json`.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },
}

pub fn run(command: SetupCommand) -> Result<Value, CorvusError> {
    match command {
        SetupCommand::Claude { remove, yes, file } => claude(remove, yes, file),
    }
}

fn io_error(path: &Path, e: &std::io::Error) -> CorvusError {
    CorvusError::Internal(format!("{}: {e}", path.display()))
}

/// The user settings file: `$CLAUDE_CONFIG_DIR/settings.json` when Claude Code is pointed elsewhere, else
/// `~/.claude/settings.json`.
fn default_settings_path() -> Result<PathBuf, CorvusError> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("settings.json"));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".claude").join("settings.json"))
        .ok_or_else(|| CorvusError::Validation("HOME is not set; pass --file <path>".into()))
}

fn read_optional(path: &Path) -> Result<Option<String>, CorvusError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path, &e)),
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

fn confirmed() -> bool {
    let _ = write!(std::io::stderr(), "Write this change? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn write_through_temp(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = sibling(path, ".corvus-tmp");
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        if let Ok(meta) = std::fs::metadata(path) {
            f.set_permissions(meta.permissions())?;
        }
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn claude(remove: bool, yes: bool, file: Option<PathBuf>) -> Result<Value, CorvusError> {
    let shown = match file {
        Some(f) => f,
        None => default_settings_path()?,
    };
    // Write through a symlink (dotfile managers), never over it.
    let path = std::fs::canonicalize(&shown).unwrap_or_else(|_| shown.clone());
    let exe = std::env::current_exe()
        .map_err(|e| CorvusError::Internal(format!("cannot find the corvus binary's own path: {e}")))?;
    let program = claude_setup::hook_program(&exe)?;
    let original = read_optional(&path)?;
    let change = claude_setup::plan(original.as_deref(), &program, remove)?;

    if !change.changed {
        say(&if remove {
            format!("Nothing to remove: {} has no Corvus hooks.", shown.display())
        } else {
            format!("Already set up: {} has the Corvus hooks.", shown.display())
        });
        return Ok(json!({ "file": path, "changed": false }));
    }

    say(&if remove {
        format!("Removing the Corvus hooks from {}:\n", shown.display())
    } else {
        format!("Adding the Corvus hooks to {}:\n", shown.display())
    });
    say(claude_setup::diff(&change.before, &change.after).trim_end());
    say("");
    if original.as_deref().is_some_and(|t| t != change.before) {
        say("The rest of the file keeps its content and key order; it is re-indented with two spaces.");
    }
    let backup = original.as_ref().map(|_| sibling(&path, ".corvus-backup"));
    if let Some(b) = &backup {
        say(&format!("The current file is copied to {} first.", b.display()));
    }
    if !yes && !confirmed() {
        say("Nothing was changed.");
        return Ok(json!({ "file": path, "changed": false, "declined": true }));
    }

    // The file may have been edited while the question was open: never write over that.
    if read_optional(&path)? != original {
        return Err(CorvusError::Conflict(format!(
            "{} changed while waiting; nothing was written — run the command again",
            path.display()
        )));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
    }
    if let Some(b) = &backup {
        std::fs::copy(&path, b).map_err(|e| io_error(b, &e))?;
    }
    write_through_temp(&path, &change.after).map_err(|e| io_error(&path, &e))?;

    if remove {
        say("Removed. Running sessions stop using the hooks at their next event.");
    } else {
        say("Done. Running sessions pick the hooks up at their next event.");
        say(&format!(
            "\nOne more thing, by hand: a woken session stops at a permission prompt unless the Corvus tools are\n\
             allowed. With the MCP server registered as \"corvus\"\n\
             (claude mcp add --scope user corvus -- {program} mcp), add this to the same file:\n\n  \
             \"permissions\": {{ \"allow\": [\"{}\"] }}",
            claude_setup::ALLOW_RULE
        ));
    }
    Ok(json!({ "file": path, "changed": true, "removed": remove, "backup": backup }))
}
