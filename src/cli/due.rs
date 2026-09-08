use std::fmt;

use anki::scheduler::parse_due_date_str;
use anyhow::{Result, bail};
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::notes;
use crate::output;
use crate::session::AnkiResultExt;

const DAYS_HELP: &str = "\
DAYS uses the desktop's syntax:
  0      today
  1      tomorrow
  3-7    a random day 3 to 7 days from now
  1!     tomorrow, and set the interval to 1 day as well (FSRS ignores the !)

Cards keep their review history and memory state; new cards become review cards.";

#[derive(Args)]
#[command(after_help = DAYS_HELP)]
pub struct DueArgs {
    /// Days from today: 0, 3-7, or 1! (see below).
    #[arg(value_name = "DAYS")]
    days: String,

    /// Note ids, or "-" to read them from stdin.
    #[arg(value_name = "NOTE_ID", required = true)]
    ids: Vec<String>,
}

#[derive(Serialize)]
struct Rescheduled {
    days: String,
    notes: usize,
    cards: usize,
}

pub fn run(ctx: &Context, args: DueArgs) -> Result<()> {
    // rslib reports a bad spec as just the spec itself; check first for a real message.
    if parse_due_date_str(&args.days).is_err() {
        bail!(
            "{:?} is not a number of days; use 0 for today, 3-7 for a random day in that range, or 1! to also set the interval",
            args.days
        );
    }
    let nids = notes::note_ids(&args.ids)?;
    let mut session = ctx.open()?;
    let cids = notes::card_ids(&mut session.col, &nids)?;
    session
        .col
        .set_due_date(&cids, &args.days, None)
        .ctx("setting due date")?;
    session.close()?;
    output::emit(
        &Rescheduled {
            days: args.days,
            notes: nids.len(),
            cards: cids.len(),
        },
        ctx.json,
    )
}

impl fmt::Display for Rescheduled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (days, force_interval) = match self.days.strip_suffix('!') {
            Some(days) => (days, true),
            None => (self.days.as_str(), false),
        };
        write!(f, "{} card(s) of {} note(s) due ", self.cards, self.notes)?;
        if days == "0" {
            write!(f, "today")?;
        } else {
            write!(f, "in {days} day(s)")?;
        }
        if force_interval {
            write!(f, ", interval set to match")?;
        }
        writeln!(f)
    }
}
