//! `corvus watch`, `corvus agent …`, `corvus adapter …` (plan 0007). Parse, call core, serialise: the logic is
//! in `corvus_core::delivery::{watch, adapters, bindings}`.

use std::time::Duration;

use clap::{Args, Subcommand};
use corvus_core::delivery::adapters::{self, CHECK_TIMEOUT};
use corvus_core::delivery::bindings;
use corvus_core::delivery::watch::{DEFAULT_TIMEOUT, Wake, WatchTimings, wait_for_unread, watch_loop};
use corvus_core::{Config, Corvus};
use corvus_domain::{AgentName, CorvusError};
use serde_json::{Value, json};

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// The agent whose inbox to watch.
    pub agent: String,
    /// Print unread messages as JSON once there are any, then exit (the default for an unbound agent).
    #[arg(long, conflicts_with = "exec")]
    pub print: bool,
    /// Seconds a wake command may run before its process group is killed (default: 300, or the adapter's own).
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: Option<u64>,
    /// Run CMD [ARGS...] for each batch of new unread messages (never through a shell). Must come last.
    #[arg(long, num_args = 1.., allow_hyphen_values = true, value_name = "CMD")]
    pub exec: Option<Vec<String>>,
}

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Record which tool an agent runs in, so it can be woken.
    Bind {
        agent: String,
        #[arg(long)]
        adapter: String,
        /// The tool's conversation/session id, if its adapter needs one.
        #[arg(long)]
        conversation: Option<String>,
        /// Replace an existing binding.
        #[arg(long)]
        force: bool,
    },
    /// Remove an agent's binding.
    Unbind { agent: String },
    /// List bindings.
    List,
}

#[derive(Subcommand, Debug)]
pub enum AdapterCommand {
    /// List built-in and user adapters.
    List,
    /// Show one parsed adapter.
    Show { name: String },
    /// Is the tool installed? Runs the adapter's check command.
    Check { name: String },
}

fn runtime() -> Result<tokio::runtime::Runtime, CorvusError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CorvusError::Internal(format!("cannot start the async runtime: {e}")))
}

/// Sets the stop flag on SIGINT or SIGTERM.
fn stop_on_signals(tx: tokio::sync::watch::Sender<bool>) {
    use tokio::signal::unix::{SignalKind, signal};
    tokio::spawn(async move {
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()))
        else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        let _ = tx.send(true);
    });
}

pub fn watch(config: &Config, args: WatchArgs) -> Result<Value, CorvusError> {
    let agent = AgentName::parse(&args.agent)?;
    let timeout = args.timeout.map(Duration::from_secs);
    let wake = if let Some(argv) = args.exec {
        Some(Wake::exec(argv, timeout.unwrap_or(DEFAULT_TIMEOUT))?)
    } else if args.print {
        None
    } else {
        match bindings::get(&config.state_dir, agent.as_str())? {
            Some(b) => {
                let loaded = adapters::load(&config.state_dir, &b.adapter)?;
                Some(Wake::adapter(loaded.adapter, b.conversation, timeout))
            }
            None => None,
        }
    };
    let (corvus, _) = Corvus::open(config)?;
    let timings = WatchTimings::default();
    runtime()?.block_on(async {
        let (tx, rx) = tokio::sync::watch::channel(false);
        stop_on_signals(tx);
        match wake {
            None => match wait_for_unread(&corvus, &agent, timings.recheck, rx).await? {
                Some(messages) => serde_json::to_value(messages)
                    .map_err(|e| CorvusError::Internal(format!("cannot encode messages: {e}"))),
                None => Ok(Value::Null),
            },
            Some(wake) => {
                watch_loop(&corvus, &config.state_dir, &agent, wake, timings, rx).await?;
                Ok(Value::Null)
            }
        }
    })
}

pub fn agent(config: &Config, c: AgentCommand) -> Result<Value, CorvusError> {
    let state = &config.state_dir;
    match c {
        AgentCommand::Bind {
            agent,
            adapter,
            conversation,
            force,
        } => {
            let loaded = adapters::load(state, &adapter)?;
            let b = bindings::bind(state, &agent, &loaded.adapter, conversation.as_deref(), force)?;
            Ok(json!({ "agent": agent, "adapter": b.adapter, "conversation": b.conversation }))
        }
        AgentCommand::Unbind { agent } => {
            bindings::unbind(state, &agent)?;
            Ok(json!({ "agent": agent, "unbound": true }))
        }
        AgentCommand::List => Ok(Value::Array(
            bindings::list(state)?
                .into_iter()
                .map(|(agent, b)| json!({ "agent": agent, "adapter": b.adapter, "conversation": b.conversation }))
                .collect(),
        )),
    }
}

pub fn adapter(config: &Config, c: AdapterCommand) -> Result<Value, CorvusError> {
    let state = &config.state_dir;
    match c {
        AdapterCommand::List => serde_json::to_value(adapters::list(state))
            .map_err(|e| CorvusError::Internal(format!("cannot encode adapters: {e}"))),
        AdapterCommand::Show { name } => Ok(adapters::to_json(&adapters::load(state, &name)?)),
        AdapterCommand::Check { name } => Ok(adapters::check(
            &adapters::load(state, &name)?.adapter,
            CHECK_TIMEOUT,
        )),
    }
}
