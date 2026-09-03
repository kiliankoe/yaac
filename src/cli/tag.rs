use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::Context;
use crate::notes;
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Subcommand)]
pub enum TagCommand {
    /// Add tags to notes.
    Add(TagArgs),
    /// Remove tags from notes.
    Remove(TagArgs),
}

#[derive(Args)]
pub struct TagArgs {
    /// One or more tags, space-separated.
    #[arg(value_name = "TAGS")]
    tags: String,

    /// Note ids, or "-" to read them from stdin.
    #[arg(value_name = "NOTE_ID", required = true)]
    ids: Vec<String>,
}

#[derive(Serialize)]
struct Tagged {
    action: &'static str,
    tags: String,
    /// Notes whose tags actually changed.
    notes: usize,
}

pub fn run(ctx: &Context, command: TagCommand) -> Result<()> {
    let (action, args) = match command {
        TagCommand::Add(args) => ("added", args),
        TagCommand::Remove(args) => ("removed", args),
    };
    let nids = notes::note_ids(&args.ids)?;
    let mut session = ctx.open()?;
    let changed = match action {
        "added" => session
            .col
            .add_tags_to_notes(&nids, &args.tags)
            .ctx("adding tags")?,
        _ => session
            .col
            .remove_tags_from_notes(&nids, &args.tags)
            .ctx("removing tags")?,
    };
    session.close()?;
    output::emit(
        &Tagged {
            action,
            tags: args.tags,
            notes: changed.output,
        },
        ctx.json,
    )
}

impl fmt::Display for Tagged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} {:?} on {} note(s)",
            self.action, self.tags, self.notes
        )
    }
}
