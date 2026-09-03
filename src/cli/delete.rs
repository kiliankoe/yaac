use std::fmt;
use std::io::{IsTerminal, Write};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::notes::{self, NoteTable, NoteView};
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Args)]
pub struct DeleteArgs {
    /// Note ids, or "-" to read them from stdin.
    #[arg(value_name = "NOTE_ID", required = true)]
    ids: Vec<String>,

    /// Skip the confirmation prompt. Required when stdin is not a terminal.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Serialize)]
struct Deleted {
    notes: usize,
    cards: usize,
}

pub fn run(ctx: &Context, args: DeleteArgs) -> Result<()> {
    let nids = notes::note_ids(&args.ids)?;
    let mut session = ctx.open()?;
    // Looking the notes up first turns a mistyped id into an error instead of a no-op,
    // and gives the prompt something to show.
    let views = notes::views(&mut session.col, &nids)?;
    if !args.yes && !confirm(&views)? {
        bail!("aborted");
    }
    // Deletions are undoable only within a process, so once we return they are final
    // and sync propagates them.
    let removed = session.col.remove_notes(&nids).ctx("deleting notes")?;
    session.close()?;
    output::emit(
        &Deleted {
            notes: nids.len(),
            cards: removed.output,
        },
        ctx.json,
    )
}

fn confirm(views: &[NoteView]) -> Result<bool> {
    let cards: usize = views.iter().map(|note| note.cards.len()).sum();
    eprint!("{}", NoteTable(views));
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        bail!("refusing to delete without --yes when not running interactively");
    }
    eprint!(
        "Delete {} note(s) and their {} card(s)? [y/N] ",
        views.len(),
        cards
    );
    std::io::stderr().flush()?;
    let mut answer = String::new();
    stdin
        .read_line(&mut answer)
        .context("reading confirmation")?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

impl fmt::Display for Deleted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "deleted {} note(s) and {} card(s)",
            self.notes, self.cards
        )
    }
}
