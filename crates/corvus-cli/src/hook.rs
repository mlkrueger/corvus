//! `corvus hook` entry point: build the hook's environment cheaply, run it inside `catch_unwind`, and print
//! its output or nothing. Every failure is silent and the process exits 0 ([system/06](../../../spec/system/06-delivery-helpers.md)).

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use corvus_core::Config;
use corvus_core::delivery::hook::{HookEnv, run};

pub fn main() {
    let out = catch_unwind(AssertUnwindSafe(|| {
        let mut stdin = String::new();
        std::io::stdin().take(1 << 20).read_to_string(&mut stdin).ok()?;
        let config = Config::load().ok()?;
        if std::env::var_os("CORVUS_LOG").is_some() {
            corvus_core::logging::init(&config.log);
        }
        let env = HookEnv {
            agent: config.agent_name.as_ref().map(ToString::to_string),
            state_dir: config.state_dir.clone(),
            database_path: config.database_path.clone(),
            operator_name: config.operator_name.to_string(),
            throttle_ms: i64::try_from(config.hook_throttle_seconds).unwrap_or(5) * 1000,
            host_pid: host_pid(),
        };
        run(&stdin, &env)
    }));
    if let Ok(Some(text)) = out {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{text}");
    }
}

/// The `claude` process this hook belongs to: `CLAUDE_PID`, else the parent (hooks are its direct children).
fn host_pid() -> Option<u32> {
    std::env::var("CLAUDE_PID")
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .filter(|p| *p > 1)
        .or_else(|| Some(std::os::unix::process::parent_id()).filter(|p| *p > 1))
}
