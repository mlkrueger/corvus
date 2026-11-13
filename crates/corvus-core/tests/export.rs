//! Plan 0003: `export_state_to_git` contents, path safety and commit scoping.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

use std::path::Path;
use std::process::Command;

use corvus_core::api::{
    CreateTask, CreateThread, ExportRequest, PostMessage, RecordDecision, RegisterSession, WriteDigest,
};
use corvus_core::testing::{TestDb, query_i64};
use corvus_core::{Services, Settings};
use corvus_domain::{CorvusError, MessageType};

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn temp_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    git(d.path(), &["config", "user.email", "test@example.invalid"]);
    git(d.path(), &["config", "user.name", "Corvus Test"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    d
}

fn seeded(repo: Option<&Path>) -> (TestDb, Services) {
    let t = TestDb::new();
    let s = t
        .services_with(Settings::default())
        .with_export_repo(repo.map(Path::to_path_buf));
    for a in ["alice", "bob"] {
        s.sessions()
            .register(&RegisterSession {
                agent_name: Some(a.into()),
                client_name: "t".into(),
                ..Default::default()
            })
            .unwrap();
    }
    let task = s
        .huginn()
        .create_task(&CreateTask {
            creator: "alice".into(),
            title: "Ship export".into(),
            ..Default::default()
        })
        .unwrap();
    let th = s
        .muninn()
        .create_thread(&CreateThread {
            creator: "alice".into(),
            title: "t".into(),
            ..Default::default()
        })
        .unwrap();
    s.muninn()
        .post_message(&PostMessage {
            thread_id: th.id,
            from: "alice".into(),
            body: "hello bob".into(),
            kind: MessageType::Note,
            to: Some("bob".into()),
            in_reply_to: None,
            artifacts: vec![],
            idempotency_key: None,
            session_id: None,
        })
        .unwrap();
    for (scope, r) in [
        ("global", None),
        ("task", Some(task.id.clone())),
        ("agent", Some("bob".to_owned())),
    ] {
        s.muninn()
            .write_digest(&WriteDigest {
                author: "alice".into(),
                scope: scope.into(),
                reference: r,
                summary: format!("{scope} summary"),
                ..Default::default()
            })
            .unwrap();
    }
    s.muninn()
        .record_decision(&RecordDecision {
            decided_by: "alice".into(),
            title: "Use SQLite".into(),
            decision: "embedded".into(),
            ..Default::default()
        })
        .unwrap();
    (t, s)
}

#[test]
fn no_repo_configured_is_a_validation_error() {
    let (_t, s) = seeded(None);
    assert!(matches!(
        s.export().export_state_to_git(&ExportRequest::default()),
        Err(CorvusError::Validation(_))
    ));
}

#[test]
fn bad_subdirs_rejected() {
    let repo = temp_repo();
    let (_t, s) = seeded(Some(repo.path()));
    for bad in ["/abs", "..", "a/../../b", "../x", "a\0b", "  "] {
        let r = s.export().export_state_to_git(&ExportRequest {
            subdir: Some(bad.into()),
            commit: false,
        });
        assert!(matches!(r, Err(CorvusError::Validation(_))), "{bad:?}");
    }
    assert_eq!(
        std::fs::read_dir(repo.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name() != ".git")
            .count(),
        0
    );
}

#[test]
fn writes_all_files_with_expected_content_and_repo_relative_paths() {
    let repo = temp_repo();
    let (t, s) = seeded(Some(repo.path()));
    let r = s.export().export_state_to_git(&ExportRequest::default()).unwrap();
    let mut paths = r.paths.clone();
    paths.sort();
    let task_id = s.huginn().list_tasks(&Default::default()).unwrap()[0].id.clone();
    let mut want: Vec<String> = [
        "coordination/summary.md".to_owned(),
        "coordination/open_tasks.json".into(),
        "coordination/decisions.md".into(),
        "coordination/inbox/alice.json".into(),
        "coordination/inbox/bob.json".into(),
        "coordination/digests/global.md".into(),
        format!("coordination/digests/task-{task_id}.md"),
        "coordination/digests/agent-bob.md".into(),
    ]
    .into();
    want.sort();
    assert_eq!(paths, want);
    assert!(paths.iter().all(|p| !p.starts_with('/')));
    let read = |p: &str| std::fs::read_to_string(repo.path().join(p)).unwrap();
    assert!(read("coordination/summary.md").contains("Open tasks: 1"));
    assert!(read("coordination/summary.md").contains("Ship export"));
    let open: serde_json::Value = serde_json::from_str(&read("coordination/open_tasks.json")).unwrap();
    assert_eq!(open[0]["title"], "Ship export");
    let bob: serde_json::Value = serde_json::from_str(&read("coordination/inbox/bob.json")).unwrap();
    assert_eq!(bob[0]["body"], "hello bob");
    assert_eq!(read("coordination/inbox/alice.json").trim(), "[]");
    assert!(read("coordination/digests/global.md").contains("global summary"));
    assert!(read("coordination/decisions.md").contains("Use SQLite"));
    assert!(!r.committed);
    assert_eq!(
        query_i64(&t.db, "SELECT count(*) FROM events WHERE kind = 'export.written'"),
        Some(1)
    );
    // No temp files left behind.
    let leftovers = walk(repo.path())
        .iter()
        .filter(|p| {
            Path::new(p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(leftovers, 0);
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if e.file_name() != ".git" {
                out.extend(walk(&p));
            }
        } else {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out
}

#[test]
fn stale_inbox_file_removed() {
    let repo = temp_repo();
    let (_t, s) = seeded(Some(repo.path()));
    let stale = repo.path().join("coordination/inbox/ghost.json");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, "[]").unwrap();
    let r = s.export().export_state_to_git(&ExportRequest::default()).unwrap();
    assert!(!stale.exists());
    assert_eq!(r.removed, vec!["coordination/inbox/ghost.json"]);
}

#[test]
fn commit_includes_only_exported_paths() {
    let repo = temp_repo();
    let p = repo.path();
    std::fs::write(p.join("README.md"), "base\n").unwrap();
    git(p, &["add", "README.md"]);
    git(p, &["commit", "-q", "-m", "base"]);
    // Unrelated dirty state: a modified tracked file, a staged new file and an untracked file.
    std::fs::write(p.join("README.md"), "dirty\n").unwrap();
    std::fs::write(p.join("staged.txt"), "s\n").unwrap();
    git(p, &["add", "staged.txt"]);
    std::fs::write(p.join("untracked.txt"), "u\n").unwrap();

    let (_t, s) = seeded(Some(p));
    let r = s
        .export()
        .export_state_to_git(&ExportRequest {
            subdir: Some("coord".into()),
            commit: true,
        })
        .unwrap();
    assert!(r.committed);
    let files = git(p, &["show", "--name-only", "--pretty=format:", "HEAD"]);
    let files: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
    assert!(files.iter().all(|f| f.starts_with("coord/")), "{files:?}");
    assert_eq!(files.len(), r.paths.len());
    let status = git(p, &["status", "--porcelain"]);
    assert!(status.contains(" M README.md"), "{status}");
    assert!(status.contains("A  staged.txt"), "{status}");
    assert!(status.contains("?? untracked.txt"), "{status}");

    // Again with nothing changed: success, nothing committed.
    let again = s
        .export()
        .export_state_to_git(&ExportRequest {
            subdir: Some("coord".into()),
            commit: true,
        })
        .unwrap();
    assert!(!again.committed);
}
