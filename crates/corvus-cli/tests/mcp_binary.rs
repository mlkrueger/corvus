//! Plan 0004: the real `corvus mcp` binary over stdio, with an empty environment (no token, no server, no
//! database path), and two MCP processes on one database waking each other.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::Home;
use serde_json::{Value, json};

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl Mcp {
    fn spawn(home: &Home) -> Self {
        let mut child = home
            .corvus()
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut m = Self {
            child,
            stdin,
            lines: rx,
            next_id: 1,
        };
        let init = m.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("Mail first")
        );
        m.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        m
    }

    fn send(&mut self, v: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("response within 30 s");
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout is protocol only ({e}): {line}"));
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    /// Calls a tool and returns the parsed JSON text content plus the isError flag.
    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let res = &r["result"];
        let text = res["content"][0]["text"].as_str().unwrap_or("null");
        (serde_json::from_str(text).unwrap(), res["isError"] == json!(true))
    }

    fn close(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        self.child.wait().unwrap()
    }
}

#[test]
fn corvus_mcp_with_an_empty_environment_runs_the_agent_flow() {
    let home = Home::new();
    let mut m = Mcp::spawn(&home);
    let tools = m.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 20);

    let (c, err) = m.call(
        "register_session",
        json!({"agent_name": "worker", "client_name": "test"}),
    );
    assert!(!err, "{c}");
    assert_eq!(c["session"]["agent_name"], "worker");
    let (t, _) = m.call("create_task", json!({"title": "do it"}));
    let task = t["id"].as_str().unwrap().to_owned();
    let (claim, _) = m.call("claim_task", json!({}));
    assert_eq!(claim["task"]["id"], json!(task));
    let (done, err) = m.call("complete_task", json!({"task_id": task, "result_summary": "ok"}));
    assert!(!err, "{done}");
    assert_eq!(done["status"], "done");
    let (_, err) = m.call("end_session", json!({}));
    assert!(!err);
    let (e, err) = m.call("heartbeat", json!({}));
    assert!(err);
    assert_eq!(e["error"]["code"], "session_not_active");
    assert!(m.close().success());

    // The database was created under the default state directory, with nothing configured.
    assert!(home.db_path().is_file());
}

#[test]
fn two_mcp_processes_wait_and_wake() {
    let home = Home::new();
    let mut waiter = Mcp::spawn(&home);
    waiter.call(
        "register_session",
        json!({"agent_name": "bob", "client_name": "test"}),
    );
    let mut poster = Mcp::spawn(&home);
    poster.call(
        "register_session",
        json!({"agent_name": "alice", "client_name": "test"}),
    );

    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let r = waiter.call("wait_for_inbox", json!({"timeout_seconds": 30}));
        tx.send(r).unwrap();
        waiter.close()
    });
    std::thread::sleep(Duration::from_millis(1500));
    let (sent, err) = poster.call("send_direct_message", json!({"to": "bob", "body": "wake up"}));
    assert!(!err, "{sent}");
    let (got, err) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("waiter woke within 5 s");
    assert!(!err, "{got}");
    assert_eq!(got[0]["body"], "wake up");
    assert!(handle.join().unwrap().success());
    assert!(poster.close().success());
}
