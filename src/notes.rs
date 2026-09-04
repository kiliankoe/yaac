//! Note views shared by every command that prints notes, plus the argument parsing that
//! turns user input into note ids and field values.

use std::fmt;
use std::io::BufRead;
use std::sync::Arc;

use anki::collection::Collection;
use anki::decks::DeckId;
use anki::notes::{Note, NoteId};
use anki::notetype::{Notetype, NotetypeKind};
use anki::text::{html_to_text_line, strip_html_preserving_media_filenames};
use anki_proto::cards::Card as CardProto;
use anki_proto::notes::note_fields_check_response::State as FieldsState;
use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::config::Config;
use crate::render::latex;
use crate::session::AnkiResultExt;

#[derive(Serialize)]
pub struct NoteView {
    pub id: i64,
    pub guid: String,
    pub notetype: String,
    /// Deck of the first card; per-card decks are in `cards`.
    pub deck: String,
    pub tags: Vec<String>,
    pub modified: i64,
    /// The notetype's sort field with HTML stripped, what the browser shows in lists.
    pub sort_field: String,
    pub fields: Fields,
    pub cards: Vec<CardView>,
}

/// Field name to HTML value, in notetype order. A map in JSON, a list here so the order
/// survives serialisation without pulling in an ordered-map crate.
pub struct Fields(pub Vec<(String, String)>);

impl Serialize for Fields {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(name, value)| (name, value)))
    }
}

#[derive(Serialize)]
pub struct CardView {
    pub id: i64,
    pub template: String,
    pub deck: String,
    pub queue: &'static str,
    /// Days until due for review cards: negative means overdue. Absent for other queues.
    pub due_in_days: Option<i32>,
    pub interval_days: u32,
    pub reps: u32,
    pub lapses: u32,
    pub flag: u32,
}

impl CardView {
    /// The scheduling facts worth a glance, in display order: queue, due date,
    /// interval, reviews and lapses, flag. Zero values are left out.
    pub fn stats(&self) -> Vec<String> {
        let mut parts = vec![self.queue.to_string()];
        if let Some(days) = self.due_in_days {
            parts.push(due_phrase(days));
        }
        if self.interval_days > 0 {
            parts.push(format!("ivl {}d", self.interval_days));
        }
        if self.reps > 0 {
            parts.push(format!("reps {}  lapses {}", self.reps, self.lapses));
        }
        if self.flag > 0 {
            parts.push(format!("flag {}", flag_name(self.flag)));
        }
        parts
    }

    pub fn suspended(&self) -> bool {
        self.queue == SUSPENDED
    }
}

const SUSPENDED: &str = "suspended";

impl NoteView {
    /// The note carries Anki's "marked" tag.
    pub fn marked(&self) -> bool {
        is_marked(&self.tags)
    }

    /// The first flag any card has, in template order, or 0. Flags are per card,
    /// but a note list can only show one.
    pub fn flag(&self) -> u32 {
        self.cards
            .iter()
            .map(|card| card.flag)
            .find(|&flag| flag > 0)
            .unwrap_or(0)
    }

    /// Every card is suspended, so the note never comes up in review.
    pub fn suspended(&self) -> bool {
        !self.cards.is_empty() && self.cards.iter().all(CardView::suspended)
    }
}

/// Anki's "marked" state is this tag on the note; the desktop shows it as a star.
pub const MARKED_TAG: &str = "marked";

pub fn is_marked(tags: &[String]) -> bool {
    tags.iter().any(|tag| tag.eq_ignore_ascii_case(MARKED_TAG))
}

/// Adds or removes the "marked" tag, the way the desktop's mark toggle does.
pub fn set_marked(col: &mut Collection, nid: NoteId, marked: bool) -> Result<()> {
    if marked {
        col.add_tags_to_notes(&[nid], MARKED_TAG)
            .ctx("marking note")?;
    } else {
        col.remove_tags_from_notes(&[nid], MARKED_TAG)
            .ctx("unmarking note")?;
    }
    Ok(())
}

/// The flag after `flag` when cycling: through Anki's seven colours, then none.
pub fn next_flag(flag: u32) -> u32 {
    (flag + 1) % 8
}

/// Anki's flag colours by number; 0 is no flag.
pub fn flag_name(flag: u32) -> &'static str {
    match flag {
        1 => "red",
        2 => "orange",
        3 => "green",
        4 => "blue",
        5 => "pink",
        6 => "turquoise",
        7 => "purple",
        _ => "none",
    }
}

/// Builds views for several notes at once so the scheduler day is computed only once.
pub fn views(col: &mut Collection, nids: &[NoteId]) -> Result<Vec<NoteView>> {
    let days_elapsed = col
        .timing_today()
        .ctx("reading scheduler day")?
        .days_elapsed;
    nids.iter()
        .map(|nid| view(col, *nid, days_elapsed))
        .collect()
}

fn view(col: &mut Collection, nid: NoteId, days_elapsed: u32) -> Result<NoteView> {
    let note = get_note(col, nid)?;
    let notetype = get_notetype(col, &note)?;
    let mut cards = col.storage.all_cards_of_note(nid).ctx("reading cards")?;
    cards.sort_by_key(|card| card.template_idx());
    let cards = cards
        .into_iter()
        .map(|card| {
            let deck = deck_name(col, card.deck_id())?;
            let template = template_name(&notetype, card.template_idx());
            let card: CardProto = card.into();
            Ok(CardView {
                id: card.id,
                template,
                deck,
                queue: queue_name(card.queue),
                due_in_days: matches!(card.queue, 2 | 3).then(|| card.due - days_elapsed as i32),
                interval_days: card.interval,
                reps: card.reps,
                lapses: card.lapses,
                flag: card.flags,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let sort_field = note
        .fields()
        .get(notetype.config.sort_field_idx as usize)
        .map(|value| latex::formulas_to_text(&html_to_text_line(value, true)))
        .unwrap_or_default();
    let fields = Fields(
        notetype
            .fields
            .iter()
            .zip(note.fields())
            .map(|(field, value)| (field.name.clone(), value.clone()))
            .collect(),
    );
    Ok(NoteView {
        id: note.id.0,
        guid: note.guid.clone(),
        notetype: notetype.name.clone(),
        deck: cards
            .first()
            .map(|card| card.deck.clone())
            .unwrap_or_default(),
        tags: note.tags.clone(),
        modified: note.mtime.0,
        sort_field,
        fields,
        cards,
    })
}

pub fn get_note(col: &mut Collection, nid: NoteId) -> Result<Note> {
    col.storage
        .get_note(nid)
        .ctx("reading note")?
        .with_context(|| format!("note {} does not exist", nid.0))
}

pub fn get_notetype(col: &mut Collection, note: &Note) -> Result<Arc<Notetype>> {
    col.get_notetype(note.notetype_id)
        .ctx("reading notetype")?
        .with_context(|| format!("notetype of note {} is missing", note.id.0))
}

pub fn deck_name(col: &mut Collection, did: DeckId) -> Result<String> {
    Ok(col
        .get_deck(did)
        .ctx("reading deck")?
        .map(|deck| deck.human_name())
        .unwrap_or_else(|| format!("deck {}", did.0)))
}

fn template_name(notetype: &Notetype, idx: u16) -> String {
    if notetype.config.kind() == NotetypeKind::Cloze {
        // Cloze notetypes have a single template; the card ordinal is the cloze number.
        format!("Cloze {}", idx + 1)
    } else {
        notetype
            .templates
            .get(idx as usize)
            .map(|template| template.name.clone())
            .unwrap_or_else(|| format!("Card {}", idx + 1))
    }
}

fn queue_name(queue: i32) -> &'static str {
    match queue {
        0 => "new",
        1 | 3 => "learning",
        2 => "review",
        4 => "preview",
        -1 => SUSPENDED,
        -2 | -3 => "buried",
        _ => "unknown",
    }
}

/// Notetype from the flag, else the config default, else an error.
pub fn resolve_notetype(
    col: &mut Collection,
    name: Option<&str>,
    config: &Config,
) -> Result<Arc<Notetype>> {
    let Some(name) = name.or(config.default_notetype.as_deref()) else {
        bail!("no notetype given; pass --notetype or set default_notetype in the config");
    };
    col.get_notetype_by_name(name)
        .ctx("looking up notetype")?
        .with_context(|| format!("notetype {name:?} does not exist; see `yaac notetypes`"))
}

/// Deck from the flag, else the config default, else an error. Never creates decks.
pub fn resolve_deck(col: &mut Collection, name: Option<&str>, config: &Config) -> Result<DeckId> {
    let Some(name) = name.or(config.default_deck.as_deref()) else {
        bail!("no deck given; pass --deck or set default_deck in the config");
    };
    col.get_deck_id(name)
        .ctx("looking up deck")?
        .with_context(|| format!("deck {name:?} does not exist; see `yaac decks`"))
}

/// What Anki's pre-add checks found, short of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldsCheck {
    Ok,
    /// Another note of the notetype has the same first field.
    Duplicate,
}

/// Runs Anki's own checks on a note about to be added. An empty first field and cloze
/// markers that do not match the notetype are errors; a duplicate is reported, since
/// callers let the user allow it.
pub fn check_new_note(
    col: &mut Collection,
    note: &Note,
    notetype_name: &str,
) -> Result<FieldsCheck> {
    Ok(match col.note_fields_check(note).ctx("checking fields")? {
        FieldsState::Normal => FieldsCheck::Ok,
        FieldsState::Duplicate => FieldsCheck::Duplicate,
        FieldsState::Empty => bail!("the first field is empty"),
        FieldsState::MissingCloze => {
            bail!(
                "{notetype_name} is a cloze notetype but no field contains a {{{{c1::...}}}} marker"
            )
        }
        FieldsState::NotetypeNotCloze | FieldsState::FieldNotCloze => {
            bail!("cloze markers found but {notetype_name} is not a cloze notetype")
        }
    })
}

pub fn field_index(notetype: &Notetype, name: &str) -> Option<usize> {
    notetype
        .fields
        .iter()
        .position(|field| field.name.eq_ignore_ascii_case(name))
}

/// Field arguments are either all `NAME=VALUE`, where NAME is a field of the notetype, or
/// all bare values in the notetype's field order. Mixing the two is rejected rather than
/// guessed, because a bare value may legitimately contain `=`.
pub fn parse_field_args(notetype: &Notetype, args: &[String]) -> Result<Vec<(usize, String)>> {
    let named: Vec<Option<(usize, &str)>> = args
        .iter()
        .map(|arg| {
            arg.split_once('=')
                .and_then(|(name, value)| field_index(notetype, name).map(|idx| (idx, value)))
        })
        .collect();
    if named.iter().all(Option::is_some) {
        return Ok(named
            .into_iter()
            .flatten()
            .map(|(idx, value)| (idx, value.to_string()))
            .collect());
    }
    if named.iter().any(Option::is_some) {
        bail!(
            "mix of NAME=VALUE and bare field values; use one style (fields of {}: {})",
            notetype.name,
            field_names(notetype)
        );
    }
    if args.len() > notetype.fields.len() {
        bail!(
            "{} values given but {} has only {} fields ({})",
            args.len(),
            notetype.name,
            notetype.fields.len(),
            field_names(notetype)
        );
    }
    Ok(args.iter().cloned().enumerate().collect())
}

pub fn field_names(notetype: &Notetype) -> String {
    notetype
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Note ids from arguments, reading whitespace-separated ids from stdin for `-`.
pub fn note_ids(args: &[String]) -> Result<Vec<NoteId>> {
    let mut ids = Vec::new();
    for arg in args {
        if arg == "-" {
            for line in std::io::stdin().lock().lines() {
                let line = line.context("reading note ids from stdin")?;
                for token in line.split_whitespace() {
                    ids.push(parse_id(token)?);
                }
            }
        } else {
            ids.push(parse_id(arg)?);
        }
    }
    if ids.is_empty() {
        bail!("no note ids given");
    }
    ids.dedup();
    Ok(ids)
}

fn parse_id(token: &str) -> Result<NoteId> {
    token
        .parse::<i64>()
        .map(NoteId)
        .with_context(|| format!("{token:?} is not a note id"))
}

/// Tags may arrive as repeated flags or as one space- or comma-separated string.
pub fn split_tags(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|tags| tags.split([' ', ',']))
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn plain_text(html: &str) -> String {
    latex::formulas_to_text(strip_html_preserving_media_filenames(html).trim())
}

/// One line per note, for `search` and `add`.
#[derive(Serialize)]
#[serde(transparent)]
pub struct NoteList(pub Vec<NoteView>);

impl fmt::Display for NoteList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        NoteTable(&self.0).fmt(f)
    }
}

/// The same one-line-per-note table over borrowed views, for prompts.
pub struct NoteTable<'a>(pub &'a [NoteView]);

impl fmt::Display for NoteTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for note in self.0 {
            writeln!(
                f,
                "{:<13}  {:<14}  {:<18}  {:<40}  {}",
                note.id,
                truncate(&note.notetype, 14),
                truncate(&note.deck, 18),
                truncate(&note.sort_field, 40),
                note.tags.join(" ")
            )?;
        }
        Ok(())
    }
}

/// Every field of every note, for `show` and `edit`.
#[derive(Serialize)]
#[serde(transparent)]
pub struct NoteDetails(pub Vec<NoteView>);

impl fmt::Display for NoteDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, note) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            writeln!(f, "id        {}", note.id)?;
            writeln!(f, "notetype  {}", note.notetype)?;
            writeln!(f, "deck      {}", note.deck)?;
            writeln!(f, "tags      {}", note.tags.join(" "))?;
            for (name, value) in &note.fields.0 {
                let text = plain_text(value);
                let mut lines = text.lines();
                writeln!(f, "{:<9} {}", name, lines.next().unwrap_or_default())?;
                for line in lines {
                    writeln!(f, "          {line}")?;
                }
            }
            for card in &note.cards {
                write!(
                    f,
                    "card      {}  {}  {}",
                    card.id,
                    card.template,
                    card.stats().join("  ")
                )?;
                if card.deck != note.deck {
                    write!(f, "  deck {}", card.deck)?;
                }
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

fn due_phrase(days: i32) -> String {
    match days {
        0 => "due today".to_string(),
        d if d < 0 => format!("overdue {}d", -d),
        d => format!("due in {d}d"),
    }
}

/// At most `max` characters, the last one an ellipsis when something was cut.
pub fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        let mut shortened: String = head.chars().take(max.saturating_sub(1)).collect();
        shortened.push('…');
        shortened
    } else {
        head
    }
}
