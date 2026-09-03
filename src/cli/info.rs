use std::fmt;
use std::path::{Path, PathBuf};

use anki::search::SortMode;
use anki::timestamp::TimestampSecs;
use anyhow::{Context, Result};
use serde::Serialize;

use crate::output;
use crate::session::Session;

#[derive(Serialize)]
struct Info {
    collection: PathBuf,
    backend_version: String,
    notes: usize,
    cards: usize,
    decks: usize,
    notetypes: usize,
    due: Due,
}

/// Cards due today across all decks, after daily limits.
#[derive(Serialize)]
struct Due {
    new: u32,
    learn: u32,
    review: u32,
}

pub fn run(collection: Option<&Path>, json: bool) -> Result<()> {
    let mut session = Session::open(collection)?;
    let info = collect(&mut session)?;
    session.close()?;
    output::emit(&info, json)
}

fn collect(session: &mut Session) -> Result<Info> {
    let col = &mut session.col;
    let notes = col
        .search_notes_unordered("")
        .context("counting notes")?
        .len();
    let cards = col
        .search_cards("", SortMode::NoOrder)
        .context("counting cards")?
        .len();
    let decks = col
        .get_all_deck_names(false)
        .context("listing decks")?
        .len();
    let notetypes = col
        .storage
        .get_all_notetype_names()
        .context("listing notetypes")?
        .len();
    // The tree root carries the collection-wide counts with limits applied, the same
    // numbers the desktop shows on its deck list.
    let root = col
        .deck_tree(Some(TimestampSecs::now()))
        .context("computing due counts")?;

    Ok(Info {
        collection: session.path.clone(),
        backend_version: anki::version::version().to_string(),
        notes,
        cards,
        decks,
        notetypes,
        due: Due {
            new: root.new_count,
            learn: root.learn_count,
            review: root.review_count,
        },
    })
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Collection  {}", self.collection.display())?;
        writeln!(f, "Backend     Anki {}", self.backend_version)?;
        writeln!(f, "Notes       {}", self.notes)?;
        writeln!(f, "Cards       {}", self.cards)?;
        writeln!(f, "Decks       {}", self.decks)?;
        writeln!(f, "Notetypes   {}", self.notetypes)?;
        writeln!(
            f,
            "Due today   new {}, learn {}, review {}",
            self.due.new, self.due.learn, self.due.review
        )
    }
}
