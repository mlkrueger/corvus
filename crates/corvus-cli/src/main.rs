//! `corvus`: the one binary. Transports only parse, call core, and serialise
//! ([system/04](../../../spec/system/04-interfaces.md#cli-corvus)).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use corvus_core::api;
use corvus_core::db::{Database, migrator};
use corvus_core::{Config, Services, SystemClock};
use corvus_domain::CorvusError;
use serde_json::{Value, json};

#[derive(Parser, Debug)]
#[command(
    name = "corvus",
    version,
    about = "Local-first coordination for AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create the database if needed and apply pending migrations.
    Migrate,
    /// Open (creating and migrating if needed) the database and report its schema version.
    Health,
    /// Sessions: register, heartbeat, end, list.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// The work queue.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Register a session and print it with the bootstrap context and guidance.
    Connect {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, default_value = "cli")]
        client: String,
    },
    /// Messages.
    Msg {
        #[command(subcommand)]
        command: MsgCommand,
    },
    /// Send a direct message as the operator (CORVUS_OPERATOR_NAME) unless --from is given.
    Tell {
        agent: String,
        message: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long = "type", default_value = "note")]
        kind: String,
    },
    /// An agent's inbox.
    Inbox {
        #[command(subcommand)]
        command: InboxCommand,
    },
    /// Digests.
    Digest {
        #[command(subcommand)]
        command: DigestCommand,
    },
    /// Decisions.
    Decision {
        #[command(subcommand)]
        command: DecisionCommand,
    },
    /// The bootstrap context for an agent.
    Bootstrap { agent: String },
    /// Queue and inbox counts.
    Stats,
    /// The MCP server over stdio. Needs no server, token or port.
    Mcp,
    /// The HTTP API on 127.0.0.1 (requires CORVUS_API_TOKEN).
    Serve,
    /// Administrative commands.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    /// Start a session. Without --agent, a name like `keen-raven` is generated.
    Register {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, default_value = "cli")]
        client: String,
        /// A JSON object.
        #[arg(long)]
        metadata: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Mark a session as alive.
    Heartbeat { session_id: String },
    /// End a session, releasing any task it holds.
    End { session_id: String },
    /// List sessions with their presence.
    List {
        /// Include ended sessions.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        agent: Option<String>,
    },
}

#[derive(Args, Debug)]
struct SessionArg {
    #[arg(long = "session")]
    session_id: String,
}

#[derive(Subcommand, Debug)]
enum TaskCommand {
    /// Create a task (the creator defaults to CORVUS_OPERATOR_NAME).
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        creator: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long)]
        pool: Option<String>,
        #[arg(long)]
        recipient: Option<String>,
        /// A task id this one waits for (repeatable).
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long = "session")]
        session_id: Option<String>,
    },
    /// Claim the best claimable task for a session.
    Claim {
        #[command(flatten)]
        session: SessionArg,
        #[arg(long)]
        pool: Option<String>,
    },
    /// Extend the lease on a task you hold.
    Renew {
        task_id: String,
        #[command(flatten)]
        session: SessionArg,
    },
    /// Give a task back to the queue.
    Release {
        task_id: String,
        #[command(flatten)]
        session: SessionArg,
    },
    /// Mark a task you hold as done.
    Complete {
        task_id: String,
        #[command(flatten)]
        session: SessionArg,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Cancel a task (the canceller defaults to CORVUS_OPERATOR_NAME).
    Cancel {
        task_id: String,
        #[arg(long)]
        by: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        /// Also cancel every dependent task.
        #[arg(long)]
        cascade: bool,
        #[arg(long = "session")]
        session_id: Option<String>,
    },
    /// List tasks, newest first.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show one task.
    Get { task_id: String },
}

#[derive(Subcommand, Debug)]
enum MsgCommand {
    /// Post a message into a thread (broadcast unless --to is given).
    Post {
        #[arg(long)]
        thread: String,
        body: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long = "type", default_value = "note")]
        kind: String,
        #[arg(long)]
        in_reply_to: Option<i64>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum InboxCommand {
    /// Unread messages, oldest first. Never acks.
    Read {
        agent: String,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        after_id: Option<i64>,
    },
    /// Acknowledge a message.
    Ack {
        message_id: i64,
        #[arg(long)]
        agent: String,
    },
    /// Block until unread messages exist (or the timeout, at most 120 s), then print them.
    Wait {
        agent: String,
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        #[arg(long = "session")]
        session_id: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum DigestCommand {
    /// Write a digest: --scope global|task|thread|agent [--ref ID-or-NAME].
    Write {
        #[arg(long)]
        scope: String,
        #[arg(long = "ref")]
        reference: Option<String>,
        summary: String,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        covers_through: Option<i64>,
    },
    /// The latest digest of every scope.
    List,
}

#[derive(Subcommand, Debug)]
enum DecisionCommand {
    /// Record a decision.
    Record {
        #[arg(long)]
        title: String,
        decision: String,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        by: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        supersedes: Option<String>,
    },
    /// Current decisions, newest first.
    List {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AdminCommand {
    /// Write the coordination snapshot into CORVUS_EXPORT_REPO.
    Export {
        #[arg(long)]
        subdir: Option<String>,
        /// Also commit exactly the exported paths.
        #[arg(long)]
        commit: bool,
    },
    /// End every expired lease now.
    ExpireLeases,
    /// End every session silent longer than CORVUS_AUTO_END_SECONDS now.
    EndStale,
    /// Write a consistent copy of the database to PATH (safe while Corvus is running).
    Backup {
        /// Destination file; must not exist.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(Value::Null) => ExitCode::SUCCESS,
        Ok(v) => {
            print_json(&v);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let body = json!({ "error": { "code": e.code(), "message": e.to_string() } });
            let _ = writeln!(std::io::stderr(), "{body}");
            ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(1))
        }
    }
}

fn run(cli: Cli) -> Result<Value, CorvusError> {
    let config = Config::load().map_err(|e| CorvusError::Validation(e.to_string()))?;
    corvus_core::logging::init(&config.log);
    config
        .ensure_state_dir()
        .map_err(|e| CorvusError::Internal(format!("{}: {e}", config.state_dir.display())))?;

    match cli.command {
        Command::Health => {
            let (_db, report) = Database::open(&config.database_path, &SystemClock)?;
            Ok(json!({ "ok": true, "schema_version": report.to_version }))
        }
        Command::Migrate => {
            let (_db, report) = Database::open(&config.database_path, &SystemClock)?;
            Ok(json!({
                "ok": true,
                "database": config.database_path,
                "schema_version": report.to_version,
                "from_version": report.from_version,
                "applied": report.applied,
                "backup": report.backup,
            }))
        }
        Command::Session { command } => session_cmd(&services(&config)?, command),
        Command::Connect { agent, client } => {
            json(services(&config)?.muninn().connect(&api::RegisterSession {
                agent_name: agent,
                client_name: client,
                ..Default::default()
            }))
        }
        Command::Msg { command } => msg_cmd(&services(&config)?, &config, command),
        Command::Tell {
            agent,
            message,
            from,
            kind,
        } => json(services(&config)?.muninn().send_direct(&api::SendDirect {
            from: from.unwrap_or_else(|| config.operator_name.to_string()),
            to: agent,
            body: message,
            kind: kind.parse()?,
            idempotency_key: None,
            session_id: None,
        })),
        Command::Inbox { command } => inbox_cmd(services(&config)?, &config, command),
        Command::Digest { command } => digest_cmd(&services(&config)?, &config, command),
        Command::Decision { command } => decision_cmd(&services(&config)?, &config, command),
        Command::Bootstrap { agent } => json(services(&config)?.muninn().bootstrap(&agent)),
        Command::Stats => json(services(&config)?.overview().stats()),
        Command::Mcp => {
            let (corvus, _) = corvus_core::Corvus::open(&config)?;
            runtime()?.block_on(corvus_mcp::run_stdio(corvus, config.agent_name.clone()))?;
            Ok(Value::Null)
        }
        Command::Serve => {
            let token = config.api_token.as_ref().ok_or_else(|| {
                CorvusError::Validation(
                    "CORVUS_API_TOKEN is required to serve the HTTP API (try: openssl rand -hex 32)".into(),
                )
            })?;
            let token = corvus_http::ApiToken::new(token.expose())?;
            let (corvus, _) = corvus_core::Corvus::open(&config)?;
            runtime()?
                .block_on(corvus_http::serve(
                    corvus,
                    token,
                    config.http_port,
                    shutdown_signal(),
                ))
                .map_err(|e| CorvusError::Internal(format!("serve: {e}")))?;
            Ok(Value::Null)
        }
        Command::Admin {
            command: AdminCommand::Export { subdir, commit },
        } => json(
            services(&config)?
                .export()
                .export_state_to_git(&api::ExportRequest { subdir, commit }),
        ),
        Command::Task { command } => task_cmd(&services(&config)?, &config, command),
        Command::Admin {
            command: AdminCommand::ExpireLeases,
        } => json(services(&config)?.huginn().expire_leases()),
        Command::Admin {
            command: AdminCommand::EndStale,
        } => json(services(&config)?.sessions().end_stale()),
        Command::Admin {
            command: AdminCommand::Backup { path },
        } => {
            let path = absolute(path)?;
            let (db, _) = Database::open(&config.database_path, &SystemClock)?;
            db.backup_to(&path)?;
            Ok(json!({ "ok": true, "path": path, "schema_version": migrator::SCHEMA_VERSION }))
        }
    }
}

fn services(config: &Config) -> Result<Services, CorvusError> {
    let (db, _) = Database::open(&config.database_path, &SystemClock)?;
    Ok(Services::from_config(Arc::new(db), Arc::new(SystemClock), config))
}

fn json<T: serde::Serialize>(r: Result<T, CorvusError>) -> Result<Value, CorvusError> {
    serde_json::to_value(r?).map_err(|e| CorvusError::Internal(e.to_string()))
}

fn session_cmd(s: &Services, c: SessionCommand) -> Result<Value, CorvusError> {
    let sessions = s.sessions();
    match c {
        SessionCommand::Register {
            agent,
            client,
            metadata,
            idempotency_key,
        } => {
            let metadata = metadata
                .map(|m| serde_json::from_str(&m))
                .transpose()
                .map_err(|e| CorvusError::Validation(format!("--metadata: {e}")))?;
            json(sessions.register(&api::RegisterSession {
                agent_name: agent,
                client_name: client,
                metadata,
                idempotency_key,
            }))
        }
        SessionCommand::Heartbeat { session_id } => json(sessions.heartbeat(&session_id)),
        SessionCommand::End { session_id } => json(sessions.end(&session_id)),
        SessionCommand::List { all, agent } => json(sessions.list(&api::ListSessions {
            include_ended: all,
            agent,
        })),
    }
}

fn task_cmd(s: &Services, config: &Config, c: TaskCommand) -> Result<Value, CorvusError> {
    let h = s.huginn();
    let operator = || config.operator_name.to_string();
    match c {
        TaskCommand::Create {
            title,
            creator,
            body,
            priority,
            pool,
            recipient,
            depends_on,
            idempotency_key,
            session_id,
        } => json(h.create_task(&api::CreateTask {
            creator: creator.unwrap_or_else(operator),
            title,
            body,
            priority,
            pool,
            recipient,
            depends_on,
            artifacts: Vec::new(),
            idempotency_key,
            session_id,
        })),
        TaskCommand::Claim { session, pool } => json(h.claim_task(&api::ClaimTask {
            session_id: session.session_id,
            pool,
        })),
        TaskCommand::Renew { task_id, session } => json(h.renew_lease(&task_id, &session.session_id)),
        TaskCommand::Release { task_id, session } => json(h.release_task(&task_id, &session.session_id)),
        TaskCommand::Complete {
            task_id,
            session,
            summary,
        } => json(h.complete_task(
            &task_id,
            &api::CompleteTask {
                session_id: session.session_id,
                result_summary: summary,
            },
        )),
        TaskCommand::Cancel {
            task_id,
            by,
            reason,
            cascade,
            session_id,
        } => json(h.cancel_task(
            &task_id,
            &api::CancelTask {
                by: by.unwrap_or_else(operator),
                reason,
                cascade,
                session_id,
            },
        )),
        TaskCommand::List { status, agent, limit } => {
            let status = status.map(|st| st.parse()).transpose()?;
            json(h.list_tasks(&api::ListTasks { status, agent, limit }))
        }
        TaskCommand::Get { task_id } => json(h.get_task(&task_id)),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, CorvusError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CorvusError::Internal(format!("runtime: {e}")))
}

/// Resolves on SIGINT or SIGTERM.
async fn shutdown_signal() {
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = term => {}
    }
}

fn msg_cmd(s: &Services, config: &Config, c: MsgCommand) -> Result<Value, CorvusError> {
    match c {
        MsgCommand::Post {
            thread,
            body,
            from,
            to,
            kind,
            in_reply_to,
            idempotency_key,
        } => json(s.muninn().post_message(&api::PostMessage {
            thread_id: thread,
            from: from.unwrap_or_else(|| config.operator_name.to_string()),
            body,
            kind: kind.parse()?,
            to,
            in_reply_to,
            artifacts: Vec::new(),
            idempotency_key,
            session_id: None,
        })),
    }
}

fn inbox_cmd(s: Services, config: &Config, c: InboxCommand) -> Result<Value, CorvusError> {
    match c {
        InboxCommand::Read {
            agent,
            limit,
            after_id,
        } => json(s.muninn().read_inbox(&api::ReadInbox {
            agent,
            limit,
            after_id,
        })),
        InboxCommand::Ack { message_id, agent } => json(s.muninn().ack_message(
            message_id,
            &api::AckMessage {
                agent,
                session_id: None,
            },
        )),
        InboxCommand::Wait {
            agent,
            timeout,
            session_id,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_time()
                .build()
                .map_err(|e| CorvusError::Internal(format!("runtime: {e}")))?;
            let hermod = corvus_core::hermod::Hermod::new(&config.database_path);
            let req = api::WaitForInbox {
                agent,
                timeout_seconds: Some(timeout),
                session_id,
                limit: None,
            };
            json(rt.block_on(corvus_core::hermod::wait_for_inbox(&s, &hermod, &req)))
        }
    }
}

fn digest_cmd(s: &Services, config: &Config, c: DigestCommand) -> Result<Value, CorvusError> {
    match c {
        DigestCommand::Write {
            scope,
            reference,
            summary,
            author,
            covers_through,
        } => json(s.muninn().write_digest(&api::WriteDigest {
            author: author.unwrap_or_else(|| config.operator_name.to_string()),
            scope,
            reference,
            summary,
            covers_through,
            idempotency_key: None,
            session_id: None,
        })),
        DigestCommand::List => json(s.muninn().list_latest_digests()),
    }
}

fn decision_cmd(s: &Services, config: &Config, c: DecisionCommand) -> Result<Value, CorvusError> {
    match c {
        DecisionCommand::Record {
            title,
            decision,
            rationale,
            by,
            task,
            thread,
            supersedes,
        } => json(s.muninn().record_decision(&api::RecordDecision {
            decided_by: by.unwrap_or_else(|| config.operator_name.to_string()),
            title,
            decision,
            rationale,
            task_id: task,
            thread_id: thread,
            supersedes,
            idempotency_key: None,
            session_id: None,
        })),
        DecisionCommand::List { task, thread, all } => json(s.muninn().list_decisions(&api::ListDecisions {
            task_id: task,
            thread_id: thread,
            include_superseded: all,
            limit: None,
        })),
    }
}

fn absolute(p: PathBuf) -> Result<PathBuf, CorvusError> {
    if p.is_absolute() {
        return Ok(p);
    }
    let cwd =
        std::env::current_dir().map_err(|e| CorvusError::Internal(format!("current directory: {e}")))?;
    Ok(cwd.join(p))
}

/// JSON on stdout: pretty on a terminal, compact otherwise.
fn print_json(v: &Value) {
    let mut out = std::io::stdout().lock();
    let text = if out.is_terminal() {
        serde_json::to_string_pretty(v)
    } else {
        serde_json::to_string(v)
    };
    let _ = writeln!(out, "{}", text.unwrap_or_default());
}
