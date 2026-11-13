//! `corvus`: the one binary. Transports only parse, call core, and serialise
//! ([system/04](../../../spec/system/04-interfaces.md#cli-corvus)).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use corvus_core::db::{Database, migrator};
use corvus_core::{Config, SystemClock};
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
    /// Administrative commands.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand, Debug)]
enum AdminCommand {
    /// Write a consistent copy of the database to PATH (safe while Corvus is running).
    Backup {
        /// Destination file; must not exist.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
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
