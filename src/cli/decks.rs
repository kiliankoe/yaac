use std::fmt;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::Context;
use crate::decks::{self, DeckRow};
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Args)]
pub struct DecksArgs {
    #[command(subcommand)]
    command: Option<DecksCommand>,
}

#[derive(Subcommand)]
pub enum DecksCommand {
    /// Create a deck, and any parents a "Parent::Child" name needs.
    Create(CreateArgs),
}

#[derive(Args)]
pub struct CreateArgs {
    /// Deck name; "Parent::Child" nests it under Parent.
    #[arg(value_name = "NAME")]
    name: String,
}

#[derive(Serialize)]
#[serde(transparent)]
struct DeckList(Vec<DeckRow>);

#[derive(Serialize)]
struct Created {
    id: i64,
    /// The name as stored, which normalisation may have changed.
    name: String,
}

pub fn run(ctx: &Context, args: DecksArgs) -> Result<()> {
    match args.command {
        None => list(ctx),
        Some(DecksCommand::Create(args)) => create(ctx, args),
    }
}

fn list(ctx: &Context) -> Result<()> {
    let mut session = ctx.open()?;
    let rows = decks::rows(&mut session.col)?;
    session.close()?;
    output::emit(&DeckList(rows), ctx.json)
}

fn create(ctx: &Context, args: CreateArgs) -> Result<()> {
    if args.name.trim().is_empty() {
        bail!("deck name is empty");
    }
    let mut session = ctx.open()?;
    // rslib appends a "+" to a name already in use instead of refusing it, which would
    // turn a typo into a second deck nobody asked for.
    if session
        .col
        .get_deck_id(&args.name)
        .ctx("looking up deck")?
        .is_some()
    {
        bail!("deck {:?} already exists", args.name);
    }
    let deck = session
        .col
        .get_or_create_normal_deck(&args.name)
        .ctx("creating deck")?;
    let created = Created {
        id: deck.id.0,
        name: deck.human_name(),
    };
    session.close()?;
    output::emit(&created, ctx.json)
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

impl fmt::Display for Created {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "created deck {:?}", self.name)
    }
}
