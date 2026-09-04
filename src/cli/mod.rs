mod add;
mod browse;
mod decks;
mod delete;
mod edit;
mod info;
mod login;
mod logout;
mod notetypes;
mod review;
mod search;
mod show;
mod stats;
mod sync;
mod tag;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::session::{Session, SessionError};

#[derive(Parser)]
#[command(
    name = "yaac",
    version,
    about = "Terminal Anki client built on Anki's own backend",
    after_help = "Without a command, yaac opens the review screen."
)]
struct Cli {
    /// Path to collection.anki2. Defaults to the config value, else the single profile
    /// under the Anki data directory.
    #[arg(long, global = true, env = "YAAC_COLLECTION", value_name = "PATH")]
    collection: Option<PathBuf>,

    /// Print JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show where the collection is, what it contains, and what is due today.
    Info,
    /// Add one note from arguments, or several from JSON.
    Add(add::AddArgs),
    /// Find notes with Anki's search syntax.
    Search(search::SearchArgs),
    /// Print notes with all their fields and cards.
    Show(show::ShowArgs),
    /// Change fields of an existing note.
    Edit(edit::EditArgs),
    /// Add or remove tags on notes.
    #[command(subcommand)]
    Tag(tag::TagCommand),
    /// Delete notes and their cards.
    Delete(delete::DeleteArgs),
    /// List decks with today's due counts.
    Decks,
    /// List notetypes with their fields and card templates.
    Notetypes,
    /// Show the statistics the desktop's stats screen shows.
    Stats(stats::StatsArgs),
    /// Log in to AnkiWeb (or a self-hosted sync server) and store the session key.
    Login(login::LoginArgs),
    /// Forget the stored sync credentials.
    Logout,
    /// Sync the collection and media with AnkiWeb.
    Sync(sync::SyncArgs),
    /// Review due cards in the terminal; also what a bare `yaac` does.
    Review(review::ReviewArgs),
    /// Search notes and edit them in the terminal.
    Browse(browse::BrowseArgs),
}

/// Everything a command needs besides its own arguments.
pub struct Context {
    pub config: Config,
    pub collection: Option<PathBuf>,
    pub json: bool,
}

impl Context {
    pub fn open(&self) -> Result<Session> {
        Session::open(self.collection.as_deref(), &self.config)
    }
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let result = Config::load().and_then(|config| {
        let ctx = Context {
            config,
            collection: cli.collection,
            json: cli.json,
        };
        let command = cli
            .command
            .unwrap_or_else(|| Command::Review(review::ReviewArgs::default()));
        match command {
            Command::Info => info::run(&ctx),
            Command::Add(args) => add::run(&ctx, args),
            Command::Search(args) => search::run(&ctx, args),
            Command::Show(args) => show::run(&ctx, args),
            Command::Edit(args) => edit::run(&ctx, args),
            Command::Tag(command) => tag::run(&ctx, command),
            Command::Delete(args) => delete::run(&ctx, args),
            Command::Decks => decks::run(&ctx),
            Command::Notetypes => notetypes::run(&ctx),
            Command::Stats(args) => stats::run(&ctx, args),
            Command::Login(args) => login::run(&ctx, args),
            Command::Logout => logout::run(&ctx),
            Command::Sync(args) => sync::run(&ctx, args),
            Command::Review(args) => review::run(&ctx, args),
            Command::Browse(args) => browse::run(&ctx, args),
        }
    });
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
