mod info;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::session::SessionError;

#[derive(Parser)]
#[command(
    name = "yaac",
    version,
    about = "Terminal Anki client built on Anki's own backend"
)]
struct Cli {
    /// Path to collection.anki2. Defaults to the single profile under the Anki data directory.
    #[arg(long, global = true, env = "YAAC_COLLECTION", value_name = "PATH")]
    collection: Option<PathBuf>,

    /// Print JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show where the collection is, what it contains, and what is due today.
    Info,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Info => info::run(cli.collection.as_deref(), cli.json),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            exit_code_for(&err)
        }
    }
}

/// A locked collection gets its own exit code so scripts can tell "quit Anki" apart from
/// other failures. Clap's usage errors already exit with 2.
fn exit_code_for(err: &anyhow::Error) -> ExitCode {
    let locked = err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<SessionError>(),
            Some(SessionError::Locked(_))
        )
    });
    if locked {
        ExitCode::from(3)
    } else {
        ExitCode::FAILURE
    }
}
