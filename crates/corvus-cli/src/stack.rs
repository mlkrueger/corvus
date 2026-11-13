//! `corvus up`, `corvus down`, `corvus status` ([system/06](../../../spec/system/06-delivery-helpers.md#stack-commands)).
//!
//! The optional background stack: `corvus serve` (only when a token is configured) and one `corvus watch` per
//! bound agent, each started detached with a pid file and a log file under `<state dir>/run/`. All three
//! commands are idempotent. None of this is needed for agents to work.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use corvus_core::background::BackgroundLeader;
use corvus_core::config::ensure_private_dir;
use corvus_core::db::Database;
use corvus_core::{Config, SystemClock};
use corvus_domain::{AgentName, CorvusError};
use serde_json::{Value, json};

const HEALTH_WAIT: Duration = Duration::from_secs(10);
const STOP_WAIT: Duration = Duration::from_secs(10);

fn run_dir(config: &Config) -> PathBuf {
    config.state_dir.join("run")
}

fn io(path: &Path, e: std::io::Error) -> CorvusError {
    CorvusError::Internal(format!("{}: {e}", path.display()))
}

/// Agents with a wake binding (`corvus agent bind`).
fn bound_agents(config: &Config) -> Vec<AgentName> {
    corvus_core::delivery::bindings::list(&config.state_dir)
        .map(|m| m.keys().filter_map(|k| AgentName::parse(k).ok()).collect())
        .unwrap_or_default()
}

/// Whether a watcher holds `<state>/watch/<agent>/lock` (the lock, not a pid file, is the proof).
fn watcher_running(config: &Config, agent: &AgentName) -> bool {
    matches!(
        corvus_core::delivery::watch::WatchLock::acquire(&config.state_dir, agent),
        Err(CorvusError::Conflict(_))
    )
}

/// The pid in `file`, if that process is alive and is a `corvus` process (a stale or reused pid is ignored).
fn live_pid(file: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(file).ok()?.trim().parse().ok()?;
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    let cmdline = String::from_utf8_lossy(&out.stdout);
    (out.status.success() && cmdline.contains("corvus")).then_some(pid)
}

fn signal(pid: u32, sig: &str) -> bool {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn alive(pid: u32) -> bool {
    signal(pid, "-0")
}

/// Starts `corvus <args>` detached (own process group), stdout/stderr appended to `log`, pid to `pid_file`.
fn spawn_detached(args: &[&str], log: &Path, pid_file: &Path) -> Result<u32, CorvusError> {
    let exe =
        std::env::current_exe().map_err(|e| CorvusError::Internal(format!("current executable: {e}")))?;
    let out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| io(log, e))?;
    let err = out.try_clone().map_err(|e| io(log, e))?;
    let child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|e| CorvusError::Internal(format!("could not start corvus {}: {e}", args.join(" "))))?;
    let pid = child.id();
    std::fs::write(pid_file, pid.to_string()).map_err(|e| io(pid_file, e))?;
    // The child is detached; reap it in the background if it exits while we are still running.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(pid)
}

/// `GET /api/health` with the token over plain loopback HTTP.
fn health(port: u16, token: &str) -> Option<Value> {
    let mut s =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let req = format!(
        "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).ok()?;
    if !resp.starts_with("HTTP/1.1 200") {
        return None;
    }
    serde_json::from_str(resp.split("\r\n\r\n").nth(1)?).ok()
}

fn server_state(config: &Config) -> Value {
    let pid = live_pid(&run_dir(config).join("serve.pid"));
    let healthy = match (&config.api_token, pid) {
        (Some(t), Some(_)) => health(config.http_port, t.expose()).is_some(),
        _ => false,
    };
    json!({
        // `pid`/`running` describe a server started by `corvus up` (it has a pid file); `enabled` and `healthy`
        // need CORVUS_API_TOKEN in this command's environment.
        "running": pid.is_some(),
        "enabled": config.api_token.is_some(),
        "pid": pid,
        "healthy": healthy,
        "url": config.api_token.as_ref().map(|_| format!("http://127.0.0.1:{}", config.http_port)),
    })
}

fn watchers_state(config: &Config) -> Value {
    let run = run_dir(config);
    bound_agents(config)
        .into_iter()
        .map(|a| {
            let pid = live_pid(&run.join(format!("watch-{a}.pid")));
            json!({ "agent": a.as_str(), "pid": pid, "running": watcher_running(config, &a) })
        })
        .collect()
}

pub fn up(config: &Config) -> Result<Value, CorvusError> {
    config.ensure_state_dir().map_err(|e| io(&config.state_dir, e))?;
    let run = run_dir(config);
    ensure_private_dir(&run).map_err(|e| io(&run, e))?;
    let (_db, report) = Database::open(&config.database_path, &SystemClock)?;

    let mut notes = Vec::new();
    match &config.api_token {
        None => notes.push("HTTP API disabled: set CORVUS_API_TOKEN to run corvus serve".to_owned()),
        Some(token) => {
            let pid_file = run.join("serve.pid");
            if live_pid(&pid_file).is_none() {
                spawn_detached(&["serve"], &run.join("serve.log"), &pid_file)?;
                let started = Instant::now();
                loop {
                    if health(config.http_port, token.expose()).is_some() {
                        break;
                    }
                    if started.elapsed() > HEALTH_WAIT || live_pid(&pid_file).is_none() {
                        return Err(CorvusError::Internal(format!(
                            "corvus serve did not become healthy; see {}",
                            run.join("serve.log").display()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
    for agent in bound_agents(config) {
        let pid_file = run.join(format!("watch-{agent}.pid"));
        if !watcher_running(config, &agent) {
            spawn_detached(
                &["watch", agent.as_str()],
                &run.join(format!("watch-{agent}.log")),
                &pid_file,
            )?;
            // Report accurately: give the new watcher a moment to take its lock.
            let started = Instant::now();
            while !watcher_running(config, &agent) && started.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let mut v = status_value(config, report.to_version);
    v["notes"] = json!(notes);
    Ok(v)
}

/// Stops a process from its pid file: SIGTERM, wait, then SIGKILL. Removes the pid file.
fn stop(pid_file: &Path) -> Option<u32> {
    let pid = live_pid(pid_file);
    if let Some(pid) = pid {
        signal(pid, "-TERM");
        let started = Instant::now();
        while alive(pid) && started.elapsed() < STOP_WAIT {
            std::thread::sleep(Duration::from_millis(100));
        }
        if alive(pid) {
            signal(pid, "-KILL");
        }
    }
    let _ = std::fs::remove_file(pid_file);
    pid
}

pub fn down(config: &Config) -> Result<Value, CorvusError> {
    let run = run_dir(config);
    let mut stopped = Vec::new();
    // Watchers first, then the server. Stale pid files are cleaned up either way.
    if let Ok(entries) = std::fs::read_dir(&run) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("watch-") && name.ends_with(".pid") && stop(&e.path()).is_some() {
                stopped.push(name.trim_end_matches(".pid").to_owned());
            }
        }
    }
    if stop(&run.join("serve.pid")).is_some() {
        stopped.push("serve".to_owned());
    }
    Ok(json!({ "ok": true, "stopped": stopped }))
}

fn status_value(config: &Config, schema_version: i64) -> Value {
    json!({
        "ok": true,
        "database": config.database_path,
        "schema_version": schema_version,
        "leader": BackgroundLeader::read_info(&config.state_dir),
        "server": server_state(config),
        "watchers": watchers_state(config),
    })
}

pub fn status(config: &Config) -> Result<Value, CorvusError> {
    let version = match corvus_core::ReadOnlyDatabase::open(&config.database_path, 1000) {
        Ok(_) => corvus_core::db::migrator::SCHEMA_VERSION,
        Err(corvus_core::db::OpenError::Missing(_)) => 0,
        Err(e) => return Err(e.into()),
    };
    Ok(status_value(config, version))
}
