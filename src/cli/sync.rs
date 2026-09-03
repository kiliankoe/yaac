use std::fmt;
use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::output;
use crate::session::Session;
use crate::sync::{self, Auth, NormalOutcome};

#[derive(Args)]
pub struct SyncArgs {
    /// Replace the server's collection with this one.
    #[arg(long, conflicts_with = "full_download")]
    full_upload: bool,

    /// Replace this collection with the server's.
    #[arg(long)]
    full_download: bool,

    /// Skip the media sync.
    #[arg(long)]
    no_media: bool,

    /// Do not ask before a full upload or download.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Serialize)]
struct Report {
    collection: &'static str,
    media: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    server_message: String,
}

pub fn run(ctx: &Context, args: SyncArgs) -> Result<()> {
    let mut auth = sync::require_auth()?;
    let mut session = ctx.open()?;
    // This command is the sync; the session must not start another one on close.
    session.sync_on_close = false;

    let mut server_message = String::new();
    let direction = if args.full_upload {
        Some(true)
    } else if args.full_download {
        Some(false)
    } else {
        let report = sync::normal(&mut session, &mut auth)?;
        server_message = report.server_message;
        match report.outcome {
            NormalOutcome::Done { changed } => {
                let collection = if changed { "synced" } else { "up_to_date" };
                return finish(
                    ctx,
                    session,
                    &auth,
                    collection,
                    server_message,
                    args.no_media,
                );
            }
            NormalOutcome::FullSyncRequired {
                upload_ok,
                download_ok,
            } => Some(choose_direction(upload_ok, download_ok)?),
        }
    };

    let upload = direction.expect("a direction was chosen");
    if (args.full_upload || args.full_download) && !args.yes && !confirm(upload)? {
        bail!("aborted");
    }
    if !upload {
        // The download overwrites the file; keep what was there.
        session.backup_now()?;
    }
    let session = sync::full(session, &auth, upload)?;
    let collection = if upload { "uploaded" } else { "downloaded" };
    finish(
        ctx,
        session,
        &auth,
        collection,
        server_message,
        args.no_media,
    )
}

fn finish(
    ctx: &Context,
    mut session: Session,
    auth: &Auth,
    collection: &'static str,
    server_message: String,
    no_media: bool,
) -> Result<()> {
    let media = if no_media {
        "skipped"
    } else {
        sync::media(&mut session, auth)?;
        "synced"
    };
    session.close()?;
    sync::save_auth(auth)?;
    output::emit(
        &Report {
            collection,
            media,
            server_message,
        },
        ctx.json,
    )
}

/// Asks which side wins when the server reports that a normal sync is impossible.
fn choose_direction(upload_ok: bool, download_ok: bool) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "a full sync is required; rerun with --full-upload (server gets this collection) or --full-download (this collection gets the server's)"
        );
    }
    eprintln!("A full sync is required because the collections have diverged.");
    if upload_ok {
        eprintln!("  u  upload: replace the server's collection with this one");
    }
    if download_ok {
        eprintln!("  d  download: replace this collection with the server's");
    }
    eprintln!("  a  abort");
    loop {
        let answer = prompt("Choice: ")?;
        match answer.as_str() {
            "u" if upload_ok => return Ok(true),
            "d" if download_ok => return Ok(false),
            "a" | "" => bail!("aborted"),
            _ => eprintln!("please answer u, d, or a"),
        }
    }
}

fn confirm(upload: bool) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("refusing a full sync without --yes when not running interactively");
    }
    let what = if upload {
        "This replaces the collection on the server with the local one."
    } else {
        "This replaces the local collection with the server's."
    };
    let answer = prompt(&format!("{what} Continue? [y/N] "))?;
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

fn prompt(text: &str) -> Result<String> {
    eprint!("{text}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading input")?;
    Ok(line.trim().to_lowercase())
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let collection = match self.collection {
            "up_to_date" => "already up to date",
            "synced" => "synced",
            "uploaded" => "uploaded to the server",
            "downloaded" => "downloaded from the server",
            other => other,
        };
        writeln!(f, "collection  {collection}")?;
        writeln!(f, "media       {}", self.media)?;
        if !self.server_message.is_empty() {
            writeln!(f, "server says: {}", self.server_message)?;
        }
        Ok(())
    }
}
