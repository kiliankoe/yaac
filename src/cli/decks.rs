use std::fmt;

use anki::timestamp::TimestampSecs;
use anki_proto::decks::DeckTreeNode;
use anyhow::Result;
use serde::Serialize;

use crate::cli::Context;
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Serialize)]
#[serde(transparent)]
struct DeckList(Vec<DeckRow>);

#[derive(Serialize)]
struct DeckRow {
    id: i64,
    name: String,
    level: u32,
    new: u32,
    learn: u32,
    review: u32,
    total: u32,
}

pub fn run(ctx: &Context) -> Result<()> {
    let mut session = ctx.open()?;
    let root = session
        .col
        .deck_tree(Some(TimestampSecs::now()))
        .ctx("reading decks")?;
    session.close()?;
    let mut rows = Vec::new();
    for child in root.children {
        flatten(child, "", &mut rows);
    }
    output::emit(&DeckList(rows), ctx.json)
}

/// Tree nodes carry only the last name segment; rebuild the full "Parent::Child" name.
fn flatten(node: DeckTreeNode, parent: &str, rows: &mut Vec<DeckRow>) {
    let name = if parent.is_empty() {
        node.name.clone()
    } else {
        format!("{parent}::{}", node.name)
    };
    rows.push(DeckRow {
        id: node.deck_id,
        name: name.clone(),
        level: node.level,
        new: node.new_count,
        learn: node.learn_count,
        review: node.review_count,
        total: node.total_in_deck,
    });
    for child in node.children {
        flatten(child, &name, rows);
    }
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
            let short = deck.name.rsplit("::").next().unwrap_or(&deck.name);
            writeln!(
                f,
                "{:<40} {:>5} {:>5} {:>6} {:>6}",
                format!("{indent}{short}"),
                deck.new,
                deck.learn,
                deck.review,
                deck.total
            )?;
        }
        Ok(())
    }
}
