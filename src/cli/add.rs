use std::io::Read;
use std::path::PathBuf;

use anki::collection::Collection;
use anki::notes::Note;
use anki_proto::notes::note_fields_check_response::State as FieldsState;
use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde::Deserialize;

use crate::cli::Context;
use crate::notes::{self, NoteList};
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Args)]
pub struct AddArgs {
    /// Notetype name; falls back to default_notetype in the config.
    #[arg(long, short = 'n', value_name = "NAME")]
    notetype: Option<String>,

    /// Deck name; falls back to default_deck in the config. Decks are never created.
    #[arg(long, short = 'd', value_name = "NAME")]
    deck: Option<String>,

    /// Tag to add; repeatable, or one space-separated list.
    #[arg(long, short = 't', value_name = "TAG")]
    tag: Vec<String>,

    /// Add even if a note with the same first field exists.
    #[arg(long)]
    allow_duplicate: bool,

    /// Read notes from a JSON file, or stdin for "-": one object or an array of objects
    /// shaped {"notetype", "deck", "tags", "fields": {"Front": "..."}}. Flags and config
    /// fill in notetype, deck, and tags an object leaves out.
    #[arg(long, value_name = "PATH", conflicts_with = "fields")]
    from_json: Option<PathBuf>,

    /// Field values as NAME=VALUE, or bare values in the notetype's field order.
    /// Values are stored as HTML.
    #[arg(value_name = "FIELD")]
    fields: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteInput {
    notetype: Option<String>,
    deck: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

pub fn run(ctx: &Context, args: AddArgs) -> Result<()> {
    let mut session = ctx.open()?;
    let col = &mut session.col;
    let inputs = match &args.from_json {
        Some(path) => read_inputs(path)?,
        None => Vec::new(),
    };

    let mut added = Vec::new();
    if inputs.is_empty() {
        if args.fields.is_empty() {
            bail!("no field values given");
        }
        let notetype = notes::resolve_notetype(col, args.notetype.as_deref(), &ctx.config)?;
        let deck = notes::resolve_deck(col, args.deck.as_deref(), &ctx.config)?;
        let mut note = Note::new(&notetype);
        for (idx, value) in notes::parse_field_args(&notetype, &args.fields)? {
            note.set_field(idx, value).ctx("setting field")?;
        }
        note.tags = notes::split_tags(&args.tag);
        added.push(add(
            col,
            &mut note,
            &notetype.name,
            deck,
            args.allow_duplicate,
        )?);
    }
    for input in inputs {
        let notetype = notes::resolve_notetype(
            col,
            input.notetype.as_deref().or(args.notetype.as_deref()),
            &ctx.config,
        )?;
        let deck = notes::resolve_deck(
            col,
            input.deck.as_deref().or(args.deck.as_deref()),
            &ctx.config,
        )?;
        let mut note = Note::new(&notetype);
        for (name, value) in &input.fields {
            let idx = notes::field_index(&notetype, name).with_context(|| {
                format!(
                    "{} has no field {name:?} (fields: {})",
                    notetype.name,
                    notes::field_names(&notetype)
                )
            })?;
            let text = match value {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            note.set_field(idx, text).ctx("setting field")?;
        }
        let mut tags = notes::split_tags(&args.tag);
        tags.extend(notes::split_tags(&input.tags));
        note.tags = tags;
        added.push(add(
            col,
            &mut note,
            &notetype.name,
            deck,
            args.allow_duplicate,
        )?);
    }

    let views = notes::views(col, &added)?;
    session.close()?;
    output::emit(&NoteList(views), ctx.json)
}

fn read_inputs(path: &PathBuf) -> Result<Vec<NoteInput>> {
    let mut text = String::new();
    if path.as_os_str() == "-" {
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading JSON from stdin")?;
    } else {
        text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    }
    let value: serde_json::Value = serde_json::from_str(&text).context("parsing JSON")?;
    let inputs = match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<NoteInput>, _>>(),
        object => serde_json::from_value(object).map(|input| vec![input]),
    }
    .context("JSON must be a note object or an array of them")?;
    if inputs.is_empty() {
        bail!("JSON contains no notes");
    }
    Ok(inputs)
}

/// Runs Anki's own pre-add checks (empty first field, duplicates, cloze markers) and adds
/// the note, returning its new id.
fn add(
    col: &mut Collection,
    note: &mut Note,
    notetype_name: &str,
    deck: anki::decks::DeckId,
    allow_duplicate: bool,
) -> Result<anki::notes::NoteId> {
    match col.note_fields_check(note).ctx("checking fields")? {
        FieldsState::Normal => {}
        FieldsState::Duplicate if allow_duplicate => {}
        FieldsState::Duplicate => bail!(
            "a {notetype_name} note with the same first field already exists (use --allow-duplicate to add anyway)"
        ),
        FieldsState::Empty => bail!("the first field is empty"),
        FieldsState::MissingCloze => {
            bail!(
                "{notetype_name} is a cloze notetype but no field contains a {{{{c1::...}}}} marker"
            )
        }
        FieldsState::NotetypeNotCloze | FieldsState::FieldNotCloze => {
            bail!("cloze markers found but {notetype_name} is not a cloze notetype")
        }
    }
    col.add_note(note, deck).ctx("adding note")?;
    Ok(note.id)
}
