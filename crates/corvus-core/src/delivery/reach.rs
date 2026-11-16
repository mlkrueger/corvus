//! Who can currently be handed mail ([system/06](../../../../spec/system/06-delivery-helpers.md), "Reachable").
//!
//! The file half of the answer: which watcher locks and waiter locks are held in the state directory right
//! now. The database half (open sessions, which agent a conversation is) is the services' job; nothing here
//! opens the database.

use std::collections::BTreeSet;
use std::fs::{OpenOptions, TryLockError};
use std::path::Path;

use corvus_domain::AgentName;

use crate::delivery::hook::current_conversation;
use crate::delivery::waiter::is_waiting;

/// Lock holders found in a state directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holders {
    /// Agents with a running watcher (`watch/<agent>/lock` is held).
    pub watchers: BTreeSet<String>,
    /// Claude Code conversations whose process has a running waiter (`hook/wait/<pid>.lock` is held and
    /// `hook/current/<pid>.json` names the conversation).
    pub waiting_conversations: BTreeSet<String>,
}

/// Whether some process holds an exclusive lock on `path`. A missing file is not held.
fn held(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// Looks at every watcher and waiter lock under `state_dir`. Never fails: what cannot be read is not held.
pub fn scan(state_dir: &Path) -> Holders {
    let mut out = Holders::default();
    if let Ok(dir) = std::fs::read_dir(state_dir.join("watch")) {
        for entry in dir.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if AgentName::parse(&name).is_ok() && held(&entry.path().join("lock")) {
                out.watchers.insert(name);
            }
        }
    }
    if let Ok(dir) = std::fs::read_dir(state_dir.join("hook").join("wait")) {
        for entry in dir.flatten() {
            let pid = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".lock"))
                .and_then(|n| n.parse::<u32>().ok());
            if let Some(pid) = pid
                && is_waiting(state_dir, pid)
                && let Some(conversation) = current_conversation(state_dir, pid)
            {
                out.waiting_conversations.insert(conversation);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::hook::current_path;
    use crate::delivery::waiter::lock_path;
    use crate::delivery::watch::WatchLock;

    #[test]
    fn finds_held_locks_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        assert_eq!(scan(state), Holders::default());

        let watched = AgentName::parse("watched").unwrap();
        let lock = WatchLock::acquire(state, &watched).unwrap();
        // A watcher that has exited leaves its lock file behind, unheld.
        drop(WatchLock::acquire(state, &AgentName::parse("gone").unwrap()).unwrap());

        // A held waiter lock with a current conversation, and an unheld one.
        let session = "0a0a0a0a-1111-4222-8333-444444444444";
        for pid in [4242u32, 4343] {
            let p = lock_path(state, pid);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "1").unwrap();
            let c = current_path(state, pid);
            std::fs::create_dir_all(c.parent().unwrap()).unwrap();
            std::fs::write(&c, format!("{{\"conversation\":\"{session}-{pid}\"}}")).unwrap();
        }
        let waiter = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(state, 4242))
            .unwrap();
        waiter.try_lock().unwrap();

        let found = scan(state);
        assert_eq!(found.watchers, BTreeSet::from(["watched".to_owned()]));
        assert_eq!(
            found.waiting_conversations,
            BTreeSet::from([format!("{session}-4242")])
        );

        drop(lock);
        drop(waiter);
        assert_eq!(scan(state), Holders::default());
    }
}
