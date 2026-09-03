use std::fmt;
use std::time::Instant;

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
use crate::tui::decks::{Choice, Picker};
use crate::tui::images::{self, Images};
use crate::tui::review::Action;

#[derive(Args)]
pub struct ReviewArgs {
    /// Deck to review. Without it a picker lists every deck with today's counts.
    #[arg(value_name = "DECK")]
    deck: Option<String>,
}

#[derive(Serialize)]
struct Summary {
    decks: Vec<String>,
    answered: usize,
    seconds: u64,
}

pub fn run(ctx: &Context, args: ReviewArgs) -> Result<()> {
    let mut session = ctx.open()?;
    let mut pending = match &args.deck {
        Some(name) => Some(
            session
                .col
                .get_deck_id(name)
                .ctx("looking up deck")?
                .with_context(|| format!("deck {name:?} does not exist; see `yaac decks`"))?,
        ),
        None => None,
    };
    let mut picker = Picker::new(decks::rows(&mut session.col)?);
    let mut summary = Summary {
        decks: Vec::new(),
        answered: 0,
        seconds: 0,
    };
    let started = Instant::now();
    let media_dir = session
        .path
        .parent()
        .map(|dir| dir.join("collection.media"))
        .unwrap_or_default();
    // Probing writes to the terminal, so it happens before the alternate screen.
    let mut images = Images::new(images::probe(ctx.config.images.as_deref()), media_dir);

    let mut terminal = tui::Terminal::open();
    loop {
        let deck = match pending.take() {
            Some(deck) => deck,
            None => match tui::decks::pick(&mut terminal, &mut session, &mut picker)? {
                Choice::Deck(deck) => deck,
                Choice::Quit => break,
            },
        };
        let action = {
            let mut reviewer = Reviewer::start(&mut session.col, DeckId(deck.0))?;
            let action = tui::review::run(&mut terminal, &mut reviewer, &mut images)?;
            summary.answered += reviewer.answered;
            if !summary.decks.contains(&reviewer.deck) {
                summary.decks.push(reviewer.deck.clone());
            }
            action
        };
        picker.set_rows(decks::rows(&mut session.col)?);
        if action == Action::Quit {
            break;
        }
    }
    drop(terminal);

    summary.seconds = started.elapsed().as_secs();
    session.close()?;
    if summary.decks.is_empty() {
        return Ok(());
    }
    output::emit(&summary, ctx.json)
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}: {} card(s) answered in {:02}:{:02}",
            self.decks.join(", "),
            self.answered,
            self.seconds / 60,
            self.seconds % 60
        )
    }
}
