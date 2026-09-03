use std::fmt;

use anyhow::Result;
use serde::Serialize;

use crate::cli::Context;
use crate::decks::{self, DeckRow};
use crate::output;

#[derive(Serialize)]
#[serde(transparent)]
struct DeckList(Vec<DeckRow>);

pub fn run(ctx: &Context) -> Result<()> {
    let mut session = ctx.open()?;
    let rows = decks::rows(&mut session.col)?;
    session.close()?;
    output::emit(&DeckList(rows), ctx.json)
}

impl fmt::Display for DeckList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<40} {:>5} {:>5} {:>6} {:>6}",
            "DECK", "NEW", "LEARN", "REVIEW", "TOTAL"
        )?;
        for deck in &self.0 {
            let indent = "  ".repeat(deck.level.saturating_sub(1) as usize);
            writeln!(
                f,
                "{:<40} {:>5} {:>5} {:>6} {:>6}",
                format!("{indent}{}", deck.short_name()),
                deck.new,
                deck.learn,
                deck.review,
                deck.total
            )?;
        }
        Ok(())
    }
}
