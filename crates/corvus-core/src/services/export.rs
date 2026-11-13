//! `export_state_to_git` (brief §4.4, system/03 "Muninn: memory and export").
//!
//! One read transaction takes a consistent snapshot; it is closed before any file is touched. Each file is
//! written to a temp file in its directory, fsynced and renamed over the target. Files under `inbox/` and
//! `digests/` that are no longer produced are removed. With `commit`, exactly the exported paths are staged and
//! committed (argument vector, no shell). The audit event is written in its own short transaction afterwards.
//! Exports are snapshots: nothing ever reads them back (P1).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use corvus_domain::{CorvusError, EventKind, Result};

use crate::api::{DecisionView, DigestView, ExportRequest, ExportResult, MessageView, Stats, TaskView};
use crate::clock::format_rfc3339;
use crate::ids::random_u64;
use crate::services::{Services, overview::stats_in, views};
use crate::store::events::{self, NewEvent};
use crate::store::{decisions, digests, messages, tasks};

const DEFAULT_SUBDIR: &str = "coordination";

struct Snapshot {
    now: i64,
    stats: Stats,
    live_tasks: Vec<TaskView>,
    inboxes: Vec<(String, Vec<MessageView>)>,
    digests: Vec<DigestView>,
    decisions: Vec<DecisionView>,
}

#[derive(Debug, Clone, Copy)]
pub struct Export<'a> {
    pub(super) s: &'a Services,
}

/// `subdir` must be relative, non-empty, without `..` components or NUL.
pub fn validate_subdir(subdir: &str) -> Result<PathBuf> {
    let bad = || CorvusError::Validation("subdir: must be a relative path inside the repository".into());
    if subdir.trim().is_empty() || subdir.contains('\0') || subdir.starts_with('/') {
        return Err(bad());
    }
    let p = PathBuf::from(subdir);
    if p.components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(bad());
    }
    Ok(p)
}

impl Export<'_> {
    pub fn export_state_to_git(&self, req: &ExportRequest) -> Result<ExportResult> {
        let Some(repo) = self.s.export_repo() else {
            return Err(CorvusError::Validation(
                "no export repository: set CORVUS_EXPORT_REPO".into(),
            ));
        };
        let subdir = validate_subdir(req.subdir.as_deref().unwrap_or(DEFAULT_SUBDIR))?;
        if !repo.is_dir() {
            return Err(CorvusError::Validation(format!(
                "CORVUS_EXPORT_REPO {} is not a directory",
                repo.display()
            )));
        }
        let now = self.s.now();

        // 1. Snapshot (one read transaction), closed before any IO.
        let snap = self.s.db().read(|tx| {
            let live_tasks = tasks::live(tx)?
                .into_iter()
                .map(|r| views::task(tx, r))
                .collect::<Result<Vec<_>>>()?;
            let mut inboxes = Vec::new();
            for agent in crate::store::agents::all(tx)? {
                let msgs = messages::unread(tx, &agent, 0, 10_000)?
                    .into_iter()
                    .map(|m| views::message(tx, m))
                    .collect::<Result<Vec<_>>>()?;
                inboxes.push((agent, msgs));
            }
            Ok(Snapshot {
                now,
                stats: stats_in(tx, now)?,
                live_tasks,
                inboxes,
                digests: digests::latest_all(tx)?.into_iter().map(views::digest).collect(),
                decisions: decisions::list(tx, None, None, false, 10_000)?
                    .into_iter()
                    .map(views::decision)
                    .collect(),
            })
        })?;

        // 2. Files, outside any transaction.
        let base = repo.join(&subdir);
        let mut files: Vec<(PathBuf, String)> = vec![
            (PathBuf::from("summary.md"), summary_md(&snap)),
            (PathBuf::from("open_tasks.json"), json(&snap.live_tasks)?),
            (PathBuf::from("decisions.md"), decisions_md(&snap.decisions)),
        ];
        for (agent, msgs) in &snap.inboxes {
            files.push((PathBuf::from("inbox").join(format!("{agent}.json")), json(msgs)?));
        }
        for d in &snap.digests {
            files.push((PathBuf::from("digests").join(digest_file_name(d)), digest_md(d)));
        }
        let io = |p: &Path, e: std::io::Error| CorvusError::Internal(format!("export {}: {e}", p.display()));
        let mut written = Vec::new();
        for (rel, content) in &files {
            let target = base.join(rel);
            write_atomic(&target, content.as_bytes()).map_err(|e| io(&target, e))?;
            written.push(subdir.join(rel));
        }
        let produced: BTreeSet<PathBuf> = files.iter().map(|(rel, _)| rel.clone()).collect();
        let mut removed = Vec::new();
        for dir in ["inbox", "digests"] {
            let d = base.join(dir);
            let Ok(entries) = fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let rel = PathBuf::from(dir).join(entry.file_name());
                if entry.file_type().is_ok_and(|t| t.is_file()) && !produced.contains(&rel) {
                    fs::remove_file(entry.path()).map_err(|e| io(&entry.path(), e))?;
                    removed.push(subdir.join(rel));
                }
            }
        }

        // 3. Optional commit of exactly these paths.
        let committed = if req.commit {
            git_commit(repo, &subdir, now)?
        } else {
            false
        };

        // 4. Audit, in its own short transaction.
        let n = written.len();
        self.s.db().write(|tx| {
            events::insert(
                tx,
                EventKind::ExportWritten,
                &NewEvent {
                    summary: format!(
                        "exported {n} files to {}{}",
                        subdir.display(),
                        if committed { " and committed" } else { "" }
                    ),
                    ..NewEvent::default()
                },
                now,
            )
        })?;

        let rel = |p: &PathBuf| p.to_string_lossy().into_owned();
        Ok(ExportResult {
            paths: written.iter().map(rel).collect(),
            removed: removed.iter().map(rel).collect(),
            committed,
        })
    }
}

fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    let mut s = serde_json::to_string_pretty(v).map_err(|e| CorvusError::Internal(e.to_string()))?;
    s.push('\n');
    Ok(s)
}

/// `global.md`, `task-<uuid>.md`, `thread-<uuid>.md`, `agent-<name>.md`.
fn digest_file_name(d: &DigestView) -> String {
    match &d.reference {
        None => "global.md".to_owned(),
        Some(r) => format!("{}-{r}.md", d.scope),
    }
}

fn summary_md(s: &Snapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Corvus coordination snapshot\n");
    let _ = writeln!(
        out,
        "Generated {} (read-only snapshot; Corvus never reads this back).\n",
        format_rfc3339(s.now)
    );
    let _ = writeln!(out, "## Counts\n");
    let _ = writeln!(out, "- Open tasks: {}", s.stats.open_tasks);
    let _ = writeln!(out, "- Claimed tasks: {}", s.stats.claimed_tasks);
    let _ = writeln!(out, "- Active leases: {}", s.stats.active_leases);
    let _ = writeln!(out, "- Expired leases: {}", s.stats.expired_leases);
    let _ = writeln!(out, "- Unread messages: {}\n", s.stats.unread_total);
    let _ = writeln!(out, "## Open and claimed tasks\n");
    if s.live_tasks.is_empty() {
        let _ = writeln!(out, "None.");
    }
    for t in &s.live_tasks {
        let holder = t
            .holder
            .as_ref()
            .map(|h| format!(", held by {}", h.agent_name))
            .unwrap_or_default();
        let blocked = if t.blocked { ", blocked" } else { "" };
        let _ = writeln!(
            out,
            "- [{}] {} (priority {}, pool {}{holder}{blocked}) `{}`",
            t.status, t.title, t.priority, t.pool, t.id
        );
    }
    out
}

fn digest_md(d: &DigestView) -> String {
    let mut out = format!(
        "# Digest: {}\n\nBy {} at {}",
        d.scope_key,
        d.author,
        format_rfc3339(d.created_at.0)
    );
    if let Some(c) = d.covers_through {
        let _ = write!(out, ", covering messages through {c}");
    }
    let _ = write!(out, ".\n\n{}\n", d.summary);
    out
}

fn decisions_md(ds: &[DecisionView]) -> String {
    let mut out = String::from("# Decisions\n\nCurrent decisions, newest first (superseded ones omitted).\n");
    if ds.is_empty() {
        out.push_str("\nNone.\n");
    }
    for d in ds {
        let _ = write!(
            out,
            "\n## {}\n\n- Decided by {} at {}\n- Id `{}`\n\n{}\n",
            d.title,
            d.decided_by,
            format_rfc3339(d.created_at.0),
            d.id,
            d.decision
        );
        if !d.rationale.is_empty() {
            let _ = write!(out, "\nRationale: {}\n", d.rationale);
        }
    }
    out
}

/// Temp file in the destination directory → fsync → rename over the target.
fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{:016x}.tmp", random_u64()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Stages and commits only the export's own paths. "Nothing to commit" is success (`false`).
fn git_commit(repo: &Path, subdir: &Path, now: i64) -> Result<bool> {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .map_err(|e| CorvusError::Internal(format!("could not run git: {e}")))
    };
    // A pathspec must exist on disk or be tracked, or git refuses it (e.g. `digests/` before any digest).
    let mut specs = Vec::new();
    for p in [
        "summary.md",
        "open_tasks.json",
        "decisions.md",
        "inbox",
        "digests",
    ] {
        let spec = subdir.join(p).to_string_lossy().into_owned();
        let tracked = git(&["ls-files", "--", spec.as_str()])?;
        if repo.join(&spec).exists() || !tracked.stdout.is_empty() {
            specs.push(spec);
        }
    }
    let mut add = vec!["add", "-A", "--"];
    add.extend(specs.iter().map(String::as_str));
    let out = git(&add)?;
    if !out.status.success() {
        return Err(CorvusError::Internal(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let mut diff = vec!["diff", "--cached", "--quiet", "--"];
    diff.extend(specs.iter().map(String::as_str));
    if git(&diff)?.status.success() {
        return Ok(false);
    }
    let msg = format!("Corvus coordination snapshot {}", format_rfc3339(now));
    let mut commit = vec!["commit", "--no-verify", "-m", msg.as_str(), "--"];
    commit.extend(specs.iter().map(String::as_str));
    let out = git(&commit)?;
    if !out.status.success() {
        return Err(CorvusError::Internal(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(true)
}
