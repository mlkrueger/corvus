//! Plan 0007: `corvus up` / `down` / `status` through the real binary.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{Home, json_stdout};
use serde_json::Value;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Stack {
    home: Home,
    port: u16,
    bin: std::path::PathBuf,
}

impl Stack {
    fn new() -> Self {
        let home = Home::new();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let codex = bin.join("codex");
        std::fs::write(&codex, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            home,
            port: free_port(),
            bin,
        }
    }
    fn run(&self, token: bool, args: &[&str]) -> Value {
        let mut c = self.home.corvus();
        c.args(args)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()));
        c.env("CORVUS_HTTP_PORT", self.port.to_string());
        if token {
            c.env("CORVUS_API_TOKEN", TOKEN);
        }
        json_stdout(&c.output().unwrap())
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = self
            .home
            .corvus()
            .arg("down")
            .env("CORVUS_HTTP_PORT", self.port.to_string())
            .output();
    }
}

#[test]
fn up_without_a_token_starts_watchers_only() {
    let s = Stack::new();
    s.run(false, &["session", "register", "--agent", "bob"]);
    s.run(
        false,
        &[
            "agent",
            "bind",
            "bob",
            "--adapter",
            "codex",
            "--conversation",
            "01890000-0000-7000-8000-000000000000",
        ],
    );
    let up = s.run(false, &["up"]);
    assert_eq!(up["server"]["enabled"], false);
    assert!(up["notes"][0].as_str().unwrap().contains("HTTP API disabled"));
    assert_eq!(up["watchers"][0]["running"], true);
    assert_eq!(up["schema_version"], 6);
    let down = s.run(false, &["down"]);
    assert_eq!(down["stopped"], serde_json::json!(["watch-bob"]));
    assert_eq!(s.run(false, &["status"])["watchers"][0]["running"], false);
}

#[test]
fn up_with_a_token_starts_serve_and_is_idempotent() {
    let s = Stack::new();
    let up = s.run(true, &["up"]);
    assert_eq!(up["server"]["healthy"], true);
    let pid = up["server"]["pid"].clone();
    assert!(pid.is_number());
    let again = s.run(true, &["up"]);
    assert_eq!(again["server"]["pid"], pid, "up twice is a no-op");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let status = s.run(true, &["status"]);
    assert_eq!(status["server"]["healthy"], true);
    assert_eq!(status["database"], s.home.db_path().to_str().unwrap());
    let down = s.run(true, &["down"]);
    assert_eq!(down["stopped"], serde_json::json!(["serve"]));
    assert_eq!(
        s.run(true, &["down"])["stopped"],
        serde_json::json!([]),
        "down twice is a no-op"
    );
    assert_eq!(s.run(true, &["status"])["server"]["pid"], Value::Null);
}

#[test]
fn stale_pid_files_are_handled() {
    let s = Stack::new();
    s.run(false, &["health"]);
    let run = s.home.state_dir().join("run");
    std::fs::create_dir_all(&run).unwrap();
    // A pid that is not a running corvus process (pid 1 is launchd/init).
    std::fs::write(run.join("serve.pid"), "1").unwrap();
    assert_eq!(s.run(true, &["status"])["server"]["pid"], Value::Null);
    let up = s.run(true, &["up"]);
    assert_eq!(
        up["server"]["healthy"], true,
        "a stale pid file does not block up"
    );
    assert_ne!(up["server"]["pid"], 1);
    s.run(true, &["down"]);
    std::fs::write(run.join("serve.pid"), "999999").unwrap();
    assert_eq!(s.run(true, &["down"])["stopped"], serde_json::json!([]));
    assert!(!run.join("serve.pid").exists(), "stale pid file removed");
}
