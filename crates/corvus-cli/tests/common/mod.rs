//! Helpers for tests that spawn the built `corvus` binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A throwaway `HOME` with nothing in it.
pub struct Home {
    pub dir: tempfile::TempDir,
}

impl Home {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
    pub fn state_dir(&self) -> PathBuf {
        self.path().join(".corvus")
    }
    pub fn db_path(&self) -> PathBuf {
        self.state_dir().join("corvus.db")
    }
    /// `corvus` with an empty environment except `HOME`.
    pub fn corvus(&self) -> Command {
        corvus_in(self.path())
    }
}

/// The built binary with a cleared environment and the given `HOME`.
pub fn corvus_in(home: &Path) -> Command {
    let mut c = Command::new(assert_cmd::cargo::cargo_bin("corvus"));
    c.env_clear().env("HOME", home).env("CORVUS_LOG", "warn");
    c
}

/// `corvus` against an explicit database file.
pub fn corvus_db(home: &Path, db: &Path) -> Command {
    let mut c = corvus_in(home);
    c.env("CORVUS_DATABASE_URL", db);
    c
}

pub fn json_stdout(o: &Output) -> serde_json::Value {
    assert!(
        o.status.success(),
        "status {:?}, stderr: {}",
        o.status,
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("stdout not JSON ({e}): {:?}", o.stdout))
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
