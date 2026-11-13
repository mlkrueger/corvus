//! Plan 0001 verification through the real binary.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use common::{Home, corvus_db, json_stdout, stderr};
use corvus_core::db::{SqlResultExt, migrator};
use corvus_core::testing::{count, exec, mode, query_i64};
use corvus_core::{Database, ManualClock};

#[test]
fn zero_config_health_creates_private_state_and_migrates() {
    let home = Home::new();
    assert!(
        std::fs::read_dir(home.path()).unwrap().next().is_none(),
        "HOME starts empty"
    );
    let out = home.corvus().arg("health").output().unwrap();
    let v = json_stdout(&out);
    assert_eq!(v, serde_json::json!({ "ok": true, "schema_version": 6 }));
    assert_eq!(mode(&home.state_dir()), 0o700);
    assert_eq!(mode(&home.db_path()), 0o600);
}

#[test]
fn migrate_twice_is_a_noop() {
    let home = Home::new();
    let first = json_stdout(&home.corvus().arg("migrate").output().unwrap());
    assert_eq!(first["applied"].as_array().unwrap().len(), 6);
    assert_eq!(first["schema_version"], 6);
    let second = json_stdout(&home.corvus().arg("migrate").output().unwrap());
    assert_eq!(second["applied"], serde_json::json!([]));
    assert_eq!(second["from_version"], 6);
}

#[test]
fn many_processes_started_together_each_migration_applied_once() {
    for round in 0..3 {
        let home = Home::new();
        let children: Vec<_> = (0..16)
            .map(|_| {
                home.corvus()
                    .arg("migrate")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        let mut applied_total = 0;
        for c in children {
            let out = c.wait_with_output().unwrap();
            let v = json_stdout(&out);
            applied_total += v["applied"].as_array().unwrap().len();
            assert_eq!(v["schema_version"], 6);
        }
        assert_eq!(
            applied_total, 6,
            "round {round}: every migration applied exactly once in total"
        );
        let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
        assert_eq!(count(&db, "schema_migrations"), 6);
    }
}

#[test]
fn newer_database_refused_with_clear_message() {
    let home = Home::new();
    json_stdout(&home.corvus().arg("health").output().unwrap());
    {
        let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
        exec(&db, "PRAGMA user_version = 42").unwrap();
    }
    let out = home.corvus().arg("health").output().unwrap();
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("newer than this Corvus") && err.contains("upgrade Corvus"),
        "{err}"
    );
}

#[test]
fn explicit_database_url_is_used() {
    let home = Home::new();
    let other = tempfile::tempdir().unwrap();
    let db = other.path().join("nested/x.db");
    json_stdout(&corvus_db(home.path(), &db).arg("health").output().unwrap());
    assert!(db.is_file());
    assert_eq!(mode(db.parent().unwrap()), 0o700);
}

#[test]
fn invalid_config_fails_fast_naming_the_variable() {
    let home = Home::new();
    let out = home
        .corvus()
        .arg("health")
        .env("CORVUS_LEASE_SECONDS", "soon")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("CORVUS_LEASE_SECONDS"));
    assert!(!home.db_path().exists(), "nothing is created on a config error");
}

#[test]
fn env_file_fills_gaps_and_existing_env_wins() {
    let home = Home::new();
    let dir = home.path().join(".config/corvus");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("env");
    std::fs::write(&file, "# test\nCORVUS_HTTP_PORT=not-a-port\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();

    // The file is read: its bad value fails startup...
    let out = home.corvus().arg("health").output().unwrap();
    assert!(stderr(&out).contains("CORVUS_HTTP_PORT"), "{}", stderr(&out));
    // ...but the process environment wins over it.
    json_stdout(
        &home
            .corvus()
            .arg("health")
            .env("CORVUS_HTTP_PORT", "9000")
            .output()
            .unwrap(),
    );
}

#[test]
fn group_or_world_readable_env_file_refused() {
    let home = Home::new();
    let file = home.path().join("corvus.env");
    std::fs::write(&file, "CORVUS_LOG=info\n").unwrap();
    for m in [0o640, 0o644] {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(m)).unwrap();
        let out = home
            .corvus()
            .arg("health")
            .env("CORVUS_ENV_FILE", &file)
            .output()
            .unwrap();
        assert!(!out.status.success(), "mode {m:o}");
        assert!(stderr(&out).contains("chmod 600"), "{}", stderr(&out));
    }
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    json_stdout(
        &home
            .corvus()
            .arg("health")
            .env("CORVUS_ENV_FILE", &file)
            .output()
            .unwrap(),
    );
}

#[test]
fn admin_backup_is_consistent_while_another_process_writes() {
    let home = Home::new();
    json_stdout(&home.corvus().arg("health").output().unwrap());
    let (db, _) = Database::open(&home.db_path(), &ManualClock::default()).unwrap();
    exec(&db, "INSERT INTO agents (name, created_at) VALUES ('w', 1)").unwrap();

    // A writer that, in each transaction, inserts a task and its audit event together.
    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let writer = {
        let (path, stop, written) = (home.db_path(), stop.clone(), written.clone());
        std::thread::spawn(move || {
            let (db, _) = Database::open(&path, &ManualClock::default()).unwrap();
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                i += 1;
                let sql = format!(
                    "INSERT INTO tasks (id, title, creator, created_at, updated_at) VALUES ('t{i}', 't', 'w', 1, 1);
                     INSERT INTO events (kind, summary, task_id, created_at) VALUES ('task.created', 's', 't{i}', 1);"
                );
                db.write(|tx| tx.sql_mut().execute_batch(&sql).db()).unwrap();
                written.store(i, Ordering::Relaxed);
            }
        })
    };
    while written.load(Ordering::Relaxed) < 20 {
        std::thread::yield_now();
    }
    let dest = home.path().join("backups/b.db");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let out = home
        .corvus()
        .args(["admin", "backup"])
        .arg(&dest)
        .output()
        .unwrap();
    let v = json_stdout(&out);
    let before_stop = written.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert_eq!(v["path"], dest.to_str().unwrap());
    assert_eq!(mode(&dest), 0o600);

    let all = migrator::embedded();
    let (b, r) = Database::open_with_migrations(&dest, &all, &ManualClock::default()).unwrap();
    assert!(r.applied.is_empty());
    assert_eq!(
        query_i64(
            &b,
            "SELECT count(*) FROM pragma_integrity_check WHERE integrity_check = 'ok'"
        ),
        Some(1)
    );
    let tasks = count(&b, "tasks");
    assert!(tasks >= 20 && tasks as u64 <= before_stop + 1, "tasks {tasks}");
    // Each task and its event commit together, so a consistent snapshot has exactly as many of each.
    assert_eq!(count(&b, "events"), tasks);

    // Refuses to overwrite.
    let again = home
        .corvus()
        .args(["admin", "backup"])
        .arg(&dest)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(5));
    assert!(stderr(&again).contains("already exists"));
}

#[test]
fn version_reports_commit_and_schema() {
    let out = assert_cmd::Command::cargo_bin("corvus")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    assert!(text.contains("commit "), "{text}");
    assert!(
        text.contains(&format!("schema {}", migrator::SCHEMA_VERSION)),
        "the version string's schema must match the migrator: {text}"
    );
}
