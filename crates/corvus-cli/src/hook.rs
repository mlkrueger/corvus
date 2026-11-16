//! `corvus hook` entry point: build the hook's environment cheaply, run it inside `catch_unwind`, and print
//! its output or nothing. Every failure is silent and the process exits 0 ([system/06](../../../spec/system/06-delivery-helpers.md)).

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use corvus_core::Config;
use corvus_core::delivery::hook::{HookEnv, run};
use corvus_core::delivery::waiter::{WaitEnv, wait};

/// Reads the event and builds the hook's environment. `None` on any problem (the caller stays silent).
fn prepare() -> Option<(String, HookEnv)> {
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
    Some((stdin, env))
}

pub fn main() {
    let out = catch_unwind(AssertUnwindSafe(|| {
        let (stdin, env) = prepare()?;
        run(&stdin, &env)
    }));
    if let Ok(Some(text)) = out {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{text}");
    }
}

/// `corvus hook wait`: blocks until there is mail to wake the session with. Returns `true` when the text was
/// written to stderr and the process must exit 2 (Claude Code's wake signal); otherwise exit 0, silently.
/// `SIGTERM` / `SIGHUP` / `SIGINT` (Claude Code stopping its hooks, or the hook's time limit) end it with 0.
pub fn wait_main() -> bool {
    let out = catch_unwind(AssertUnwindSafe(|| {
        let (stdin, hook) = prepare()?;
        let headless = std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok_and(|e| !e.is_empty() && e != "cli");
        let env = WaitEnv { hook, headless };
        let stop = Arc::new(AtomicBool::new(false));
        spawn_signal_watcher(&stop);
        wait(&stdin, &env, &stop)
    }));
    match out {
        Ok(Some(text)) => {
            let mut stderr = std::io::stderr().lock();
            writeln!(stderr, "{text}").is_ok()
        }
        _ => false,
    }
}

/// Sets `stop` when the process is asked to end, so the waiter leaves through its normal path (exit 0).
fn spawn_signal_watcher(stop: &Arc<AtomicBool>) {
    use tokio::signal::unix::{SignalKind, signal};
    let stop = Arc::clone(stop);
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
            return;
        };
        rt.block_on(async {
            let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::hangup()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = hup.recv() => {}
                _ = int.recv() => {}
            }
            stop.store(true, Ordering::Relaxed);
        });
    });
}

/// The `claude` process this hook belongs to: `CLAUDE_PID`, else the parent (hooks are its direct children).
fn host_pid() -> Option<u32> {
    std::env::var("CLAUDE_PID")
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .filter(|p| *p > 1)
        .or_else(|| Some(std::os::unix::process::parent_id()).filter(|p| *p > 1))
}
