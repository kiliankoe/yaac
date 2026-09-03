use std::fmt;

use anki::decks::DeckId;
use anyhow::{Context as _, Result};
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::decks;
use crate::output;
use crate::review::Reviewer;
use crate::session::AnkiResultExt;
use crate::tui;

#[derive(Args)]
pub struct ReviewArgs {
    /// Deck to review. Without it a picker lists every deck with today's counts.
    #[arg(value_name = "DECK")]
    deck: Option<String>,
}

#[derive(Serialize)]
struct Summary {
    deck: String,
    answered: usize,
    seconds: u64,
}

pub fn run(ctx: &Context, args: ReviewArgs) -> Result<()> {
    let mut session = ctx.open()?;
    let named = match &args.deck {
        Some(name) => Some(
            session
                .col
                .get_deck_id(name)
                .ctx("looking up deck")?
                .with_context(|| format!("deck {name:?} does not exist; see `yaac decks`"))?,
        ),
        None => None,
    };
    let rows = decks::rows(&mut session.col)?;

    let summary = {
        let mut terminal = tui::Terminal::open();
        let deck = match named {
            Some(deck) => Some(deck),
            None => tui::decks::pick(&mut terminal, &rows)?,
        };
        match deck {
            None => None,
            Some(deck) => {
                let mut reviewer = Reviewer::start(&mut session.col, DeckId(deck.0))?;
                tui::review::run(&mut terminal, &mut reviewer)?;
                Some(Summary {
                    deck: reviewer.deck.clone(),
                    answered: reviewer.answered,
                    seconds: reviewer.elapsed().as_secs(),
                })
            }
        }
    };
    session.close()?;
    match summary {
        Some(summary) => output::emit(&summary, ctx.json),
        None => Ok(()),
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}: {} card(s) answered in {:02}:{:02}",
            self.deck,
            self.answered,
            self.seconds / 60,
            self.seconds % 60
        )
    }
}
