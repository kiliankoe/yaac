//! Deck list with today's counts, shared by the `decks` command and the review picker.

use anki::collection::Collection;
use anki::timestamp::TimestampSecs;
use anki_proto::decks::DeckTreeNode;
use anyhow::Result;
use serde::Serialize;

use crate::session::AnkiResultExt;

#[derive(Serialize, Clone)]
pub struct DeckRow {
    pub id: i64,
    /// Full name, "Parent::Child".
    pub name: String,
    pub level: u32,
    pub new: u32,
    pub learn: u32,
    pub review: u32,
    pub total: u32,
}

impl DeckRow {
    pub fn short_name(&self) -> &str {
        self.name.rsplit("::").next().unwrap_or(&self.name)
    }

    pub fn due(&self) -> u32 {
        self.new + self.learn + self.review
    }
}

/// Every deck in tree order with counts after daily limits, as the desktop shows them.
pub fn rows(col: &mut Collection) -> Result<Vec<DeckRow>> {
    let root = col
        .deck_tree(Some(TimestampSecs::now()))
        .ctx("reading decks")?;
    let mut rows = Vec::new();
    for child in root.children {
        flatten(child, "", &mut rows);
    }
    Ok(rows)
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
