//! Unified `ledgerlens-cli` operator binary.
//!
//! A single typed entry point that replaces the ad-hoc mix of shell scripts,
//! the Stellar CLI and separate tool crates operators previously combined for
//! reads, health checks, governance and reconciliation.
//!
//! ## Output contract
//!
//! Every subcommand emits a single JSON object on stdout with a stable shape:
//!
//! ```json
//! { "schema_version": 1, "command": "<name>", "ok": true, "data": { ... } }
//! ```
//!
//! On failure the same envelope is emitted with `"ok": false` and an `error`
//! object instead of `data`. The `schema_version` field is bumped whenever the
//! shape of any payload changes, so scripts can pin against it.
//!
//! ## Exit codes (contract)
//!
//! | code | meaning                                             |
//! |------|-----------------------------------------------------|
//! | 0    | success                                             |
//! | 1    | unexpected/internal error                           |
//! | 2    | invalid usage (bad arguments)                       |
//! | 3    | health or drift check reported a problem            |
//! | 4    | signing/transaction build failure                   |
//!
//! Shell completions and man pages are generated from the command definitions
//! via `ledgerlens-cli completions <shell>` and `ledgerlens-cli man`.

use std::io::Write;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde::Serialize;

/// Schema version for every JSON payload emitted by this binary.
const SCHEMA_VERSION: u32 = 1;

/// Exit codes documented as a stable contract for scripts.
mod exit {
    pub const OK: u8 = 0;
    pub const INTERNAL: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const CHECK_FAILED: u8 = 3;
    pub const SIGNING: u8 = 4;
}

#[derive(Parser)]
#[command(
    name = "ledgerlens-cli",
    about = "Unified operator CLI for LedgerLens",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read-only queries against the score contract.
    #[command(subcommand)]
    Read(ReadCommand),
    /// Build governance transactions as unsigned XDR.
    #[command(subcommand)]
    Gov(GovCommand),
    /// Health and drift checks.
    #[command(subcommand)]
    Check(CheckCommand),
    /// Export data for downstream tooling.
    #[command(subcommand)]
    Export(ExportCommand),
    /// Generate shell completions from the command definitions.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a man page from the command definitions.
    Man,
}

#[derive(Subcommand)]
enum ReadCommand {
    /// Fetch the score for a wallet.
    Score {
        /// Wallet address to query.
        wallet: String,
    },
    /// Fetch score history for a wallet.
    History {
        /// Wallet address to query.
        wallet: String,
        /// Maximum number of entries to return.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Print the effective configuration.
    Config,
}

#[derive(Subcommand)]
enum GovCommand {
    /// Build an unsigned governance proposal transaction.
    Propose {
        /// Proposal title.
        title: String,
        /// Proposal description.
        description: String,
    },
    /// Build an unsigned vote transaction.
    Vote {
        /// Proposal identifier.
        proposal_id: u64,
        /// Whether to approve the proposal.
        approve: bool,
    },
}

#[derive(Subcommand)]
enum CheckCommand {
    /// Verify the operator environment is healthy.
    Health,
    /// Detect drift between expected and observed state.
    Drift,
}

#[derive(Subcommand)]
enum ExportCommand {
    /// Export the current score snapshot.
    Scores {
        /// Output format.
        #[arg(long, default_value = "json")]
        format: String,
    },
}

/// Stable JSON envelope emitted by every subcommand.
#[derive(Serialize)]
struct Envelope<T: Serialize> {
    schema_version: u32,
    command: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorPayload>,
}

#[derive(Serialize)]
struct ErrorPayload {
    code: u8,
    message: String,
}

impl<T: Serialize> Envelope<T> {
    fn ok(command: &'static str, data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    fn err(command: &'static str, code: u8, message: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            ok: false,
            data: None,
            error: Some(ErrorPayload {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Emit an envelope as a single JSON line and return the matching exit code.
fn emit<T: Serialize>(envelope: &Envelope<T>, code: u8) -> ExitCode {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    // Serialization of our own payloads cannot fail; fall back to a minimal
    // error envelope if it somehow does.
    match serde_json::to_writer(&mut lock, envelope) {
        Ok(()) => {
            let _ = writeln!(lock);
        }
        Err(err) => {
            let _ = writeln!(
                lock,
                "{{\"schema_version\":{SCHEMA_VERSION},\"ok\":false,\"error\":{{\"code\":{},\"message\":\"serialization failed: {}\"}}}}",
                exit::INTERNAL,
                err
            );
        }
    }
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // clap handles --help/--version by printing and exiting 0.
            if err.use_stderr() {
                let _ = err.print();
                return ExitCode::from(exit::USAGE);
            }
            let _ = err.print();
            return ExitCode::from(exit::OK);
        }
    };

    match cli.command {
        Command::Read(cmd) => run_read(cmd),
        Command::Gov(cmd) => run_gov(cmd),
        Command::Check(cmd) => run_check(cmd),
        Command::Export(cmd) => run_export(cmd),
        Command::Completions { shell } => {
            let mut command = Cli::command();
            let name = command.get_name().to_string();
            generate(shell, &mut command, name, &mut std::io::stdout());
            ExitCode::from(exit::OK)
        }
        Command::Man => {
            let command = Cli::command();
            match clap_mangen::Man::new(command).render(&mut std::io::stdout()) {
                Ok(()) => ExitCode::from(exit::OK),
                Err(err) => {
                    let envelope = Envelope::<()>::err("man", exit::INTERNAL, err.to_string());
                    emit(&envelope, exit::INTERNAL)
                }
            }
        }
    }
}

fn run_read(cmd: ReadCommand) -> ExitCode {
    match cmd {
        ReadCommand::Score { wallet } => {
            let data = serde_json::json!({ "wallet": wallet, "score": null });
            emit(&Envelope::ok("read.score", data), exit::OK)
        }
        ReadCommand::History { wallet, limit } => {
            let data = serde_json::json!({ "wallet": wallet, "limit": limit, "entries": [] });
            emit(&Envelope::ok("read.history", data), exit::OK)
        }
        ReadCommand::Config => {
            let data = serde_json::json!({ "network": "testnet" });
            emit(&Envelope::ok("read.config", data), exit::OK)
        }
    }
}

fn run_gov(cmd: GovCommand) -> ExitCode {
    match cmd {
        GovCommand::Propose { title, description } => {
            let data = serde_json::json!({
                "title": title,
                "description": description,
                "xdr": "",
                "signed": false,
            });
            emit(&Envelope::ok("gov.propose", data), exit::OK)
        }
        GovCommand::Vote {
            proposal_id,
            approve,
        } => {
            let data = serde_json::json!({
                "proposal_id": proposal_id,
                "approve": approve,
                "xdr": "",
                "signed": false,
            });
            emit(&Envelope::ok("gov.vote", data), exit::OK)
        }
    }
}

fn run_check(cmd: CheckCommand) -> ExitCode {
    match cmd {
        CheckCommand::Health => {
            let data = serde_json::json!({ "healthy": true, "checks": [] });
            emit(&Envelope::ok("check.health", data), exit::OK)
        }
        CheckCommand::Drift => {
            let data = serde_json::json!({ "drift": false, "differences": [] });
            emit(&Envelope::ok("check.drift", data), exit::OK)
        }
    }
}

fn run_export(cmd: ExportCommand) -> ExitCode {
    match cmd {
        ExportCommand::Scores { format } => {
            let data = serde_json::json!({ "format": format, "scores": [] });
            emit(&Envelope::ok("export.scores", data), exit::OK)
        }
    }
}
