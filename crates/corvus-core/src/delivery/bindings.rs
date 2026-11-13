//! Agent bindings: which tool each agent runs in, and the identifier needed to wake it
//! ([system/06](../../../../spec/system/06-delivery-helpers.md#bindings)).
//!
//! `$STATE/agents/bindings.json` is `{ "<agent>": { "adapter": "<name>", "conversation": "<id>" | null } }`.
//! Writes are temp-file + rename with mode 0600. One binding per agent; re-binding needs `force`.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use corvus_domain::{Adapter, AgentName, CorvusError, Result};
use serde::{Deserialize, Serialize};

use crate::config::ensure_private_dir;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub adapter: String,
    pub conversation: Option<String>,
}

pub fn bindings_path(state_dir: &Path) -> PathBuf {
    state_dir.join("agents").join("bindings.json")
}

/// All bindings. A missing file is no bindings; an unreadable or malformed one is an error naming the file.
pub fn list(state_dir: &Path) -> Result<BTreeMap<String, Binding>> {
    let path = bindings_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            CorvusError::Validation(format!("{}: not a valid bindings file: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(CorvusError::Internal(format!("{}: {e}", path.display()))),
    }
}

pub fn get(state_dir: &Path, agent: &str) -> Result<Option<Binding>> {
    Ok(list(state_dir)?.remove(agent))
}

/// Binds `agent` to `adapter` (already loaded by the caller). Validates the agent name and the binding values
/// against the adapter. An existing binding is a conflict unless `force`.
pub fn bind(
    state_dir: &Path,
    agent: &str,
    adapter: &Adapter,
    conversation: Option<&str>,
    force: bool,
) -> Result<Binding> {
    let agent = AgentName::parse(agent)?;
    adapter.validate_binding(conversation)?;
    let mut all = list(state_dir)?;
    if all.contains_key(agent.as_str()) && !force {
        return Err(CorvusError::Conflict(format!(
            "{agent} is already bound; use --force to re-bind"
        )));
    }
    let binding = Binding {
        adapter: adapter.name.clone(),
        conversation: conversation.map(str::to_owned),
    };
    all.insert(agent.to_string(), binding.clone());
    save(state_dir, &all)?;
    Ok(binding)
}

pub fn unbind(state_dir: &Path, agent: &str) -> Result<()> {
    let mut all = list(state_dir)?;
    if all.remove(agent).is_none() {
        return Err(CorvusError::NotFound(format!("{agent} has no binding")));
    }
    save(state_dir, &all)
}

fn save(state_dir: &Path, all: &BTreeMap<String, Binding>) -> Result<()> {
    let path = bindings_path(state_dir);
    let bytes = serde_json::to_vec_pretty(all).map_err(|e| CorvusError::Internal(e.to_string()))?;
    write_private_atomic(&path, &bytes).map_err(|e| CorvusError::Internal(format!("{}: {e}", path.display())))
}

/// Writes `bytes` to `path` via a sibling temp file (mode 0600, fsynced) and an atomic rename. Creates the
/// parent directory (0700) if needed.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("path has no parent directory"))?;
    ensure_private_dir(dir)?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp{}", std::process::id()));
    let tmp = dir.join(tmp_name);
    let written = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}
