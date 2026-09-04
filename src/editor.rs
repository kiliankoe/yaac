//! The `$EDITOR` round-trip: a note becomes a text file with one heading per field, the
//! user edits it, and the result goes back through rslib.
//!
//! Fields stay HTML; only `<br>` is shown as a line break, so plain notes edit like plain
//! text while images, styling, and occlusion markup survive untouched. A field whose
//! text did not change is written back byte-identical.

use std::path::Path;
use std::process::Command;

use anki::collection::Collection;
use anki::decks::DeckId;
use anki::notes::{Note, NoteId};
use anki::notetype::Notetype;
use anyhow::{Context, Result, bail};

use crate::notes::{self, FieldsCheck};
use crate::session::AnkiResultExt;

const ERROR_PREFIX: &str = "<!-- yaac error:";

/// A note's editable parts, fields in notetype order as name and HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub tags: Vec<String>,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    Unchanged,
    /// The user emptied the file.
    Aborted,
}

impl Outcome {
    pub fn message(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Unchanged => "no changes",
            Self::Aborted => "aborted",
        }
    }
}

/// The command that opens a file for editing.
#[derive(Debug, Clone)]
pub struct Editor(String);

impl Editor {
    pub fn new(command: impl Into<String>) -> Self {
        Self(command.into())
    }

    /// `$VISUAL`, else `$EDITOR`, else `vi`.
    pub fn from_env() -> Self {
        let command = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "vi".to_string());
        Self(command)
    }

    /// Opens `path` and waits. Goes through `sh -c` so commands with arguments, like
    /// `code --wait`, work the way they do for git.
    pub fn open(&self, path: &Path) -> Result<()> {
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"$1\"", self.0))
            .arg("sh")
            .arg(path)
            .status()
            .with_context(|| format!("running editor {:?}", self.0))?;
        if !status.success() {
            bail!("editor {:?} exited with {status}", self.0);
        }
        Ok(())
    }
}

/// Opens the note in the editor and saves the result. Fields and tags go through
/// `update_note`, so the change is undoable and cards are regenerated as needed.
pub fn edit_note(col: &mut Collection, nid: NoteId, editor: &Editor) -> Result<Outcome> {
    let mut note = notes::get_note(col, nid)?;
    let notetype = notes::get_notetype(col, &note)?;
    let original = Draft::from_note(&note, &notetype);
    let description = format!("note {} ({})", nid.0, notetype.name);
    let Some(edited) = edit_draft(&original, &description, editor, &mut |_| Ok(()))? else {
        return Ok(Outcome::Aborted);
    };
    if edited == original {
        return Ok(Outcome::Unchanged);
    }
    apply(col, &mut note, &edited)?;
    Ok(Outcome::Saved)
}

pub fn apply(col: &mut Collection, note: &mut Note, draft: &Draft) -> Result<()> {
    draft.fill(note)?;
    col.update_note(note).ctx("updating note")?;
    Ok(())
}

/// Opens an empty note of the notetype in the editor and adds what comes back to the
/// deck. Anki's checks run on every save: an empty first field or cloze markers that do
/// not fit reopen the file with the problem on top, and so does a duplicate, once, so
/// that saving again unchanged adds it anyway. `None` when the user emptied the file.
pub fn add_note(
    col: &mut Collection,
    deck: DeckId,
    notetype: &Notetype,
    editor: &Editor,
) -> Result<Option<NoteId>> {
    let empty = Draft::empty(notetype);
    let description = format!("new {} note", notetype.name);
    let mut warned_about: Option<Draft> = None;
    let edited = edit_draft(&empty, &description, editor, &mut |draft| {
        let note = draft.to_note(notetype)?;
        if notes::check_new_note(col, &note, &notetype.name)? == FieldsCheck::Duplicate
            && warned_about.as_ref() != Some(draft)
        {
            warned_about = Some(draft.clone());
            bail!(
                "a {} note with the same first field already exists (save again without changes to add it anyway)",
                notetype.name
            );
        }
        Ok(())
    })?;
    let Some(draft) = edited else {
        return Ok(None);
    };
    let mut note = draft.to_note(notetype)?;
    col.add_note(&mut note, deck).ctx("adding note")?;
    Ok(Some(note.id))
}

/// Writes the draft to a temporary file, opens the editor, and parses what comes back.
/// A file that does not parse, or that `validate` rejects, is reopened with the error
/// at the top, until it passes or the user empties it. `None` means the user aborted.
pub fn edit_draft(
    draft: &Draft,
    description: &str,
    editor: &Editor,
    validate: &mut dyn FnMut(&Draft) -> Result<()>,
) -> Result<Option<Draft>> {
    let path = std::env::temp_dir().join(format!("yaac-edit-{}.md", std::process::id()));
    let result = edit_file(draft, description, editor, &path, validate);
    // Best effort: a leftover file in the temp dir is harmless.
    let _ = std::fs::remove_file(&path);
    result
}

fn edit_file(
    draft: &Draft,
    description: &str,
    editor: &Editor,
    path: &Path,
    validate: &mut dyn FnMut(&Draft) -> Result<()>,
) -> Result<Option<Draft>> {
    let mut text = draft.to_text(description);
    loop {
        std::fs::write(path, &text).with_context(|| format!("writing {}", path.display()))?;
        editor.open(path)?;
        let edited =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let problem = match draft.parse(&edited) {
            Ok(None) => return Ok(None),
            Ok(Some(parsed)) => match validate(&parsed) {
                Ok(()) => return Ok(Some(parsed)),
                Err(err) => err,
            },
            Err(err) => err,
        };
        text = format!(
            "{ERROR_PREFIX} {problem:#}. Fix it and save again, or empty the file to abort. -->\n{}",
            strip_error_comment(&edited)
        );
    }
}

fn strip_error_comment(text: &str) -> &str {
    match text.split_once('\n') {
        Some((first, rest)) if first.starts_with(ERROR_PREFIX) => rest,
        _ => text,
    }
}

impl Draft {
    pub fn from_note(note: &Note, notetype: &Notetype) -> Self {
        Self {
            tags: note.tags.clone(),
            fields: notetype
                .fields
                .iter()
                .zip(note.fields())
                .map(|(field, value)| (field.name.clone(), value.clone()))
                .collect(),
        }
    }

    /// No tags and every field of the notetype empty, for a new note.
    pub fn empty(notetype: &Notetype) -> Self {
        Self {
            tags: Vec::new(),
            fields: notetype
                .fields
                .iter()
                .map(|field| (field.name.clone(), String::new()))
                .collect(),
        }
    }

    /// A new note of the notetype with the draft's fields and tags.
    pub fn to_note(&self, notetype: &Notetype) -> Result<Note> {
        let mut note = Note::new(notetype);
        self.fill(&mut note)?;
        Ok(note)
    }

    /// Writes the fields and tags into `note`.
    fn fill(&self, note: &mut Note) -> Result<()> {
        for (idx, (_, html)) in self.fields.iter().enumerate() {
            note.set_field(idx, html.clone()).ctx("setting field")?;
        }
        note.tags = self.tags.clone();
        Ok(())
    }

    /// The file the user edits: a comment with instructions, the tags, then one
    /// markdown heading per field. The `.md` name gets editors to highlight both the
    /// headings and any HTML inside the fields.
    pub fn to_text(&self, description: &str) -> String {
        let mut text = format!(
            "<!-- yaac: {description}. Save and quit to apply, empty the file to abort. -->\n\
             tags: {}\n",
            self.tags.join(" ")
        );
        for (name, html) in &self.fields {
            let body = editable(html);
            if body.is_empty() {
                // One blank line to type into, rather than three.
                text.push_str(&format!("\n# {name}\n\n"));
            } else {
                text.push_str(&format!("\n# {name}\n\n{body}\n"));
            }
        }
        text
    }

    /// Reads an edited file back against this draft. Blank lines around a field's text
    /// are ignored, a field whose heading was removed keeps its value, and a field whose
    /// text is unchanged keeps its exact HTML. `Ok(None)` for an emptied file.
    pub fn parse(&self, text: &str) -> Result<Option<Draft>> {
        let text = text.replace("\r\n", "\n");
        if text.trim().is_empty() {
            return Ok(None);
        }
        let mut tags = Vec::new();
        let mut sections: Vec<(usize, Vec<&str>)> = Vec::new();
        for line in text.lines() {
            if let Some(heading) = line.strip_prefix("# ") {
                let name = heading.trim();
                let Some(idx) = self.fields.iter().position(|(field, _)| field == name) else {
                    bail!(
                        "unknown field \"{name}\" (fields: {})",
                        self.fields
                            .iter()
                            .map(|(field, _)| field.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                };
                if sections.iter().any(|(seen, _)| *seen == idx) {
                    bail!("field \"{name}\" appears twice");
                }
                sections.push((idx, Vec::new()));
                continue;
            }
            match sections.last_mut() {
                Some((_, lines)) => lines.push(line),
                None => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with("<!--") {
                        continue;
                    }
                    if let Some(rest) = trimmed.strip_prefix("tags:") {
                        tags = rest.split_whitespace().map(str::to_string).collect();
                        continue;
                    }
                    bail!("unexpected line before the first field: {trimmed:?}");
                }
            }
        }
        let mut fields = self.fields.clone();
        for (idx, lines) in sections {
            let edited = trim_blank_lines(&lines).join("\n");
            let original = &self.fields[idx].1;
            if edited != editable(original).trim_matches('\n') {
                fields[idx].1 = to_html(&edited);
            }
        }
        Ok(Some(Draft { tags, fields }))
    }
}

fn trim_blank_lines<'a, 'b>(lines: &'b [&'a str]) -> &'b [&'a str] {
    let blank = |line: &&str| line.trim().is_empty();
    let start = lines
        .iter()
        .position(|line| !blank(line))
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !blank(line))
        .map_or(start, |i| i + 1);
    &lines[start..end]
}

/// HTML with every `<br>` turned into a line break.
fn editable(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find('<') {
        out.push_str(&rest[..pos]);
        let tag = &rest[pos..];
        match br_len(tag) {
            Some(len) => {
                out.push('\n');
                rest = &tag[len..];
            }
            None => {
                out.push('<');
                rest = &tag[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Length of a `<br>` at the start of `s` in any of its spellings, else None.
fn br_len(s: &str) -> Option<usize> {
    if !s.get(..3)?.eq_ignore_ascii_case("<br") {
        return None;
    }
    let after = s[3..].trim_start_matches([' ', '/']);
    after.starts_with('>').then(|| s.len() - after.len() + 1)
}

fn to_html(text: &str) -> String {
    text.replace('\n', "<br>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        Draft {
            tags: vec!["vocab".into(), "animals".into()],
            fields: vec![
                ("Front".into(), "el gato<br>la gata".into()),
                ("Back".into(), "<b>cat</b><br/>".into()),
            ],
        }
    }

    #[test]
    fn the_file_has_tags_and_a_heading_per_field_with_br_as_newlines() {
        let text = draft().to_text("note 1 (Basic)");
        assert_eq!(
            text,
            "<!-- yaac: note 1 (Basic). Save and quit to apply, empty the file to abort. -->\n\
             tags: vocab animals\n\
             \n# Front\n\nel gato\nla gata\n\
             \n# Back\n\n<b>cat</b>\n\n"
        );
    }

    #[test]
    fn empty_fields_get_a_heading_and_one_blank_line() {
        let draft = Draft {
            tags: Vec::new(),
            fields: vec![
                ("Front".into(), String::new()),
                ("Back".into(), String::new()),
            ],
        };
        let text = draft.to_text("new Basic note");
        assert!(
            text.ends_with("tags: \n\n# Front\n\n\n# Back\n\n"),
            "{text:?}"
        );
        assert_eq!(draft.parse(&text).unwrap().unwrap(), draft);
    }

    #[test]
    fn an_untouched_file_parses_back_to_the_same_draft() {
        let original = draft();
        let parsed = original.parse(&original.to_text("x")).unwrap().unwrap();
        assert_eq!(parsed, original, "trailing <br> survives unchanged");
    }

    #[test]
    fn edited_text_becomes_html_with_br_and_other_fields_keep_their_html() {
        let original = draft();
        let text = "tags: vocab\n\n# Front\n\nthe cat\nsecond line\n\n\n# Back\n\n<b>cat</b>\n";
        let parsed = original.parse(text).unwrap().unwrap();
        assert_eq!(parsed.tags, vec!["vocab"]);
        assert_eq!(parsed.fields[0].1, "the cat<br>second line");
        assert_eq!(
            parsed.fields[1].1, "<b>cat</b><br/>",
            "same text, so the original HTML stays"
        );
    }

    #[test]
    fn a_removed_heading_keeps_the_field_and_an_empty_file_aborts() {
        let original = draft();
        let parsed = original.parse("tags:\n\n# Back\n\ndog\n").unwrap().unwrap();
        assert!(parsed.tags.is_empty());
        assert_eq!(parsed.fields[0].1, "el gato<br>la gata");
        assert_eq!(parsed.fields[1].1, "dog");
        assert_eq!(original.parse("  \n\n").unwrap(), None);
    }

    #[test]
    fn bad_headings_and_stray_header_lines_are_errors() {
        let original = draft();
        let err = original.parse("# Bakc\n\nx\n").unwrap_err().to_string();
        assert!(err.contains("unknown field \"Bakc\""), "{err}");
        assert!(err.contains("Front, Back"), "{err}");
        let err = original
            .parse("# Front\n\na\n\n# Front\n\nb\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("appears twice"), "{err}");
        let err = original
            .parse("hello\n# Front\n\nx\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("before the first field"), "{err}");
    }

    #[test]
    fn br_spellings_and_windows_line_endings_are_handled() {
        assert_eq!(editable("a<BR>b<br />c<br/>d<brx>e"), "a\nb\nc\nd<brx>e");
        let original = draft();
        let parsed = original
            .parse("tags: t\r\n\r\n# Front\r\n\r\none\r\ntwo\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.fields[0].1, "one<br>two");
    }

    #[test]
    fn the_error_comment_is_replaced_not_stacked() {
        let text = "<!-- yaac error: old -->\ntags:\n";
        assert_eq!(strip_error_comment(text), "tags:\n");
        assert_eq!(strip_error_comment("tags:\n"), "tags:\n");
    }
}
