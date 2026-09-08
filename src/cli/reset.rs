use std::fmt;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::notes;
use crate::output;
use crate::session::AnkiResultExt;

/// Same defaults as the desktop's Reset dialog: back to the original new-card position,
/// review and lapse counts untouched.
#[derive(Args)]
pub struct ResetArgs {
    /// Note ids, or "-" to read them from stdin.
    #[arg(value_name = "NOTE_ID", required = true)]
    ids: Vec<String>,

    /// Put the cards at the end of the new queue instead of back at their original position.
    #[arg(long)]
    no_restore_position: bool,

    /// Also zero the review and lapse counts.
    #[arg(long)]
    reset_counts: bool,
}

#[derive(Serialize)]
struct Reset {
    notes: usize,
    cards: usize,
}

pub fn run(ctx: &Context, args: ResetArgs) -> Result<()> {
    let nids = notes::note_ids(&args.ids)?;
    let mut session = ctx.open()?;
    let cids = notes::card_ids(&mut session.col, &nids)?;
    // Logged like the desktop does it, so the review history shows the reset.
    session
        .col
        .reschedule_cards_as_new(
            &cids,
            true,
            !args.no_restore_position,
            args.reset_counts,
            None,
        )
        .ctx("resetting cards")?;
    session.close()?;
    output::emit(
        &Reset {
            notes: nids.len(),
            cards: cids.len(),
        },
        ctx.json,
    )
}

impl fmt::Display for Reset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "reset {} card(s) of {} note(s) to new",
            self.cards, self.notes
        )
    }
}
