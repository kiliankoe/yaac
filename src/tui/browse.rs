//! The browse screen: a search box on top, matching notes below it, the selected
//! note's fields, tags, and cards under those. `e` opens the note in `$EDITOR`, `d`
//! deletes it after asking, `s`, `f`, and `m` suspend, flag, and mark it, and `t` and
//! `T` ask for tags to add or remove on the bottom line.

use std::time::Duration;

use anki::browser_table::Column;
use anki::card::CardId;
use anki::error::AnkiError;
use anki::notes::NoteId;
use anki::search::SortMode;
use anki_proto::scheduler::bury_or_suspend_cards_request::Mode as BuryOrSuspendMode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as Panel, Borders, List, ListItem, ListState, Paragraph};

use crate::editor::{self, Editor, Outcome};
use crate::notes::{self, NoteView, flag_name, truncate};
use crate::render::{Block, Stylesheet, html_to_blocks};
use crate::session::{AnkiResultExt, Session, anki_error};
use crate::tui::images::Images;
use crate::tui::{Terminal, blocks, flag_color, is_ctrl_c, next_key, overlay};

const KEYS: &[(&str, &str)] = &[
    ("/", "type a search; enter or esc leaves the box"),
    ("ctrl-u", "clear the search while typing"),
    ("j/k, ↑/↓", "move through the notes"),
    ("g/G", "first and last note"),
    ("ctrl-d/u, page down/up", "scroll the note"),
    ("e", "edit the note in $EDITOR"),
    ("d", "delete the note, after confirming"),
    (
        "s",
        "suspend the note's cards, or unsuspend them when all are",
    ),
    ("f", "cycle the flag colour on the note's cards"),
    ("m", "mark or unmark the note"),
    ("t", "add tags; tab completes from the collection's tags"),
    ("T", "remove tags; tab completes from the note's own"),
    (
        "enter",
        "run the search again, dropping notes that no longer match",
    ),
    ("u", "undo the last change"),
    ("r", "re-send and redraw the images"),
    ("esc", "back to the decks when opened from there, else quit"),
    ("q", "quit"),
];

/// What a key press asks the browse loop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum BrowseAction {
    Continue,
    /// Run the query.
    Search,
    Edit(NoteId),
    /// The user confirmed the deletion.
    Delete(NoteId),
    /// Suspend every card of the note, or unsuspend them all.
    Suspend(NoteId),
    /// Step the flag on every card of the note.
    Flag(NoteId),
    /// Mark or unmark the note.
    Mark(NoteId),
    /// Ask for tags to add to or remove from the note.
    TagPrompt(NoteId, TagMode),
    /// The tags the user entered at the prompt.
    Tags(NoteId, TagMode, Vec<String>),
    /// Run the query again unchanged.
    Rerun,
    Undo,
    /// Re-send the images.
    Redraw,
    /// Something drawn over the images went away; send their placements again.
    Refresh,
    /// Leave the screen for whatever opened it.
    Back,
    Quit,
}

/// How the screen was left: `Back` returns to the deck picker when browse was opened
/// from there, and ends the program otherwise, like `Quit`.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Back,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagMode {
    Add,
    Remove,
}

/// A question on the bottom line; it takes the keys until answered or dismissed.
enum Prompt {
    /// `d` was pressed; `y` deletes the note, any other key cancels.
    ConfirmDelete {
        nid: NoteId,
        question: String,
    },
    Tags(TagPrompt),
}

/// Tags being typed, with tab completion over `candidates`.
struct TagPrompt {
    nid: NoteId,
    mode: TagMode,
    text: String,
    /// Every tag in the collection when adding, the note's own when removing.
    candidates: Vec<String>,
    /// Tab is stepping through the matches for the word it started on.
    cycle: Option<Cycle>,
}

struct Cycle {
    matches: Vec<String>,
    index: usize,
}

impl TagPrompt {
    fn edit(&mut self, code: KeyCode, ctrl: bool) {
        match code {
            KeyCode::Tab => return self.step(1),
            KeyCode::BackTab => return self.step(-1),
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Char('u') if ctrl => self.text.clear(),
            KeyCode::Char(c) if !ctrl => self.text.push(c),
            _ => {}
        }
        // Editing the text ends a cycle; the next tab starts from what is typed.
        self.cycle = None;
    }

    /// The text before the word at the cursor, and that word: what follows the last
    /// space or comma.
    fn split(&self) -> (&str, &str) {
        match self.text.rfind([' ', ',']) {
            Some(i) => self.text.split_at(i + 1),
            None => ("", &self.text),
        }
    }

    /// Candidates starting with the word at the cursor, all of them when it is empty.
    /// Case-insensitive, as tags are in Anki.
    fn matches(&self) -> Vec<String> {
        let stem = self.split().1.to_lowercase();
        self.candidates
            .iter()
            .filter(|tag| tag.to_lowercase().starts_with(&stem))
            .cloned()
            .collect()
    }

    /// Replaces the word at the cursor with the next (or previous) match. The first
    /// tab fixes the list of matches from the word as typed, so stepping on does not
    /// narrow it to the completion just inserted.
    fn step(&mut self, by: isize) {
        let cycle = match self.cycle.take() {
            Some(mut cycle) => {
                let len = cycle.matches.len() as isize;
                cycle.index = (cycle.index as isize + by).rem_euclid(len) as usize;
                cycle
            }
            None => {
                let matches = self.matches();
                if matches.is_empty() {
                    return;
                }
                let index = if by < 0 { matches.len() - 1 } else { 0 };
                Cycle { matches, index }
            }
        };
        let head = self.split().0.to_string();
        self.text = format!("{head}{}", cycle.matches[cycle.index]);
        self.cycle = Some(cycle);
    }

    /// The prompt as drawn: label, text, cursor, then the matches for the word at the
    /// cursor, dim, with the one a cycle is on lit. Nothing is listed for an empty
    /// word until tab is pressed, since that would be every tag there is.
    fn line(&self) -> Line<'static> {
        let label = match self.mode {
            TagMode::Add => " add tags: ",
            TagMode::Remove => " remove tags: ",
        };
        let mut spans = vec![
            Span::raw(label).bold(),
            Span::raw(self.text.clone()),
            Span::raw("▏"),
        ];
        let (matches, current) = match &self.cycle {
            Some(cycle) => (cycle.matches.clone(), Some(cycle.index)),
            None if self.split().1.is_empty() => (Vec::new(), None),
            None => (self.matches(), None),
        };
        for (i, tag) in matches.into_iter().enumerate() {
            spans.push(Span::raw("  "));
            let tag = Span::raw(tag);
            spans.push(if current == Some(i) {
                tag.bold()
            } else {
                tag.dim()
            });
        }
        Line::from(spans)
    }
}

pub struct Browser {
    query: String,
    /// Keys go into the search box rather than acting as shortcuts.
    typing: bool,
    /// A search has run, so an empty list means no matches rather than no query.
    searched: bool,
    notes: Vec<NoteView>,
    list: ListState,
    /// Rows the detail pane is scrolled down by.
    scroll: u16,
    /// Content and viewport height of the detail pane at the last draw, to bound
    /// scrolling.
    detail: (u16, u16),
    status: Option<String>,
    /// The `?` overlay is up.
    help: bool,
    prompt: Option<Prompt>,
}

impl Browser {
    /// Starts in the search box when there is no query yet.
    pub fn new(query: impl Into<String>) -> Self {
        let query = query.into();
        Self {
            typing: query.trim().is_empty(),
            query,
            searched: false,
            notes: Vec::new(),
            list: ListState::default(),
            scroll: 0,
            detail: (0, 0),
            status: None,
            help: false,
            prompt: None,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn notes(&self) -> &[NoteView] {
        &self.notes
    }

    pub fn selected(&self) -> Option<&NoteView> {
        self.list.selected().and_then(|index| self.notes.get(index))
    }

    fn note(&self, nid: NoteId) -> Option<&NoteView> {
        self.notes.iter().find(|note| note.id == nid.0)
    }

    /// The selected note's id for a key that acts on it, if there is one.
    fn act_on_selected(&self, action: fn(NoteId) -> BrowseAction) -> BrowseAction {
        match self.selected() {
            Some(note) => action(NoteId(note.id)),
            None => BrowseAction::Continue,
        }
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    pub fn clear_status(&mut self) {
        self.status = None;
    }

    /// Replaces the results, keeping the selected note when it is still there.
    pub fn set_notes(&mut self, notes: Vec<NoteView>) {
        let selected = self.selected().map(|note| note.id);
        self.notes = notes;
        self.searched = true;
        let kept = selected.and_then(|id| self.notes.iter().position(|note| note.id == id));
        if kept.is_none() {
            self.scroll = 0;
        }
        self.list
            .select((!self.notes.is_empty()).then(|| kept.unwrap_or(0)));
    }

    /// Forgets the results, as for an empty query.
    pub fn clear_results(&mut self) {
        self.notes.clear();
        self.searched = false;
        self.list.select(None);
        self.scroll = 0;
    }

    /// Selects the note at `index`, or the last one when there are fewer, as after a
    /// deletion.
    pub fn select_nearest(&mut self, index: usize) {
        if let Some(last) = self.notes.len().checked_sub(1) {
            self.list.select(Some(index.min(last)));
            self.scroll = 0;
        }
    }

    /// Swaps in a fresh view of one note, after an edit.
    pub fn replace_note(&mut self, note: NoteView) {
        if let Some(existing) = self.notes.iter_mut().find(|n| n.id == note.id) {
            *existing = note;
        }
    }

    /// Puts up the tag prompt for the note, completing from `candidates`.
    pub fn open_tag_prompt(&mut self, nid: NoteId, mode: TagMode, candidates: Vec<String>) {
        self.status = None;
        self.prompt = Some(Prompt::Tags(TagPrompt {
            nid,
            mode,
            text: String::new(),
            candidates,
            cycle: None,
        }));
    }

    pub fn handle(&mut self, key: KeyEvent) -> BrowseAction {
        if is_ctrl_c(key) {
            return BrowseAction::Quit;
        }
        // The overlay swallows the key that closes it.
        if self.help {
            self.help = false;
            return BrowseAction::Refresh;
        }
        if let Some(prompt) = self.prompt.take() {
            return self.answer(prompt, key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.typing {
            // The query runs on every change; leaving the box only frees the letter
            // keys for the list. Arrows work in both modes.
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.typing = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    return BrowseAction::Search;
                }
                KeyCode::Char('u') if ctrl => {
                    self.query.clear();
                    return BrowseAction::Search;
                }
                KeyCode::Char(c) if !ctrl => {
                    self.query.push(c);
                    return BrowseAction::Search;
                }
                KeyCode::Down => self.move_selection(1),
                KeyCode::Up => self.move_selection(-1),
                KeyCode::PageDown => self.scroll_by(self.half_page()),
                KeyCode::PageUp => self.scroll_by(-self.half_page()),
                _ => {}
            }
            return BrowseAction::Continue;
        }
        match key.code {
            KeyCode::Char('q') => return BrowseAction::Quit,
            KeyCode::Esc => return BrowseAction::Back,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => {
                self.typing = true;
                self.status = None;
            }
            KeyCode::Char('e') => return self.act_on_selected(BrowseAction::Edit),
            KeyCode::Char('s') => return self.act_on_selected(BrowseAction::Suspend),
            KeyCode::Char('f') => return self.act_on_selected(BrowseAction::Flag),
            KeyCode::Char('m') => return self.act_on_selected(BrowseAction::Mark),
            KeyCode::Char('t') => {
                return self.act_on_selected(|nid| BrowseAction::TagPrompt(nid, TagMode::Add));
            }
            KeyCode::Char('T') => {
                return self.act_on_selected(|nid| BrowseAction::TagPrompt(nid, TagMode::Remove));
            }
            KeyCode::Enter => return BrowseAction::Rerun,
            KeyCode::Char('r') => return BrowseAction::Redraw,
            KeyCode::Char('d') if ctrl => self.scroll_by(self.half_page()),
            KeyCode::Char('u') if ctrl => self.scroll_by(-self.half_page()),
            KeyCode::Char('d') => {
                if let Some(note) = self.selected() {
                    let question = format!(
                        "delete \"{}\" and its {} card(s)? y confirms, any other key cancels",
                        truncate(&note.sort_field, 40),
                        note.cards.len()
                    );
                    let nid = NoteId(note.id);
                    self.status = None;
                    self.prompt = Some(Prompt::ConfirmDelete { nid, question });
                }
            }
            KeyCode::PageDown => self.scroll_by(self.half_page()),
            KeyCode::PageUp => self.scroll_by(-self.half_page()),
            KeyCode::Char('u') => return BrowseAction::Undo,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(isize::MAX / 2),
            _ => {}
        }
        BrowseAction::Continue
    }

    /// A key while a prompt is up. The prompt has been taken out of `self` and goes
    /// back unless the key answered or dismissed it.
    fn answer(&mut self, prompt: Prompt, key: KeyEvent) -> BrowseAction {
        match prompt {
            Prompt::ConfirmDelete { nid, .. } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    return BrowseAction::Delete(nid);
                }
            }
            Prompt::Tags(mut tags) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let entered = notes::split_tags(std::slice::from_ref(&tags.text));
                    if !entered.is_empty() {
                        return BrowseAction::Tags(tags.nid, tags.mode, entered);
                    }
                }
                code => {
                    tags.edit(code, key.modifiers.contains(KeyModifiers::CONTROL));
                    self.prompt = Some(Prompt::Tags(tags));
                }
            },
        }
        BrowseAction::Continue
    }

    fn move_selection(&mut self, delta: isize) {
        if self.notes.is_empty() {
            return;
        }
        let current = self.list.selected().unwrap_or(0) as isize;
        let last = self.notes.len() as isize - 1;
        let next = current.saturating_add(delta).clamp(0, last) as usize;
        self.list.select(Some(next));
        self.scroll = 0;
    }

    fn half_page(&self) -> i32 {
        i32::from(self.detail.1 / 2).max(1)
    }

    fn scroll_by(&mut self, delta: i32) {
        let max = i32::from(self.detail.0.saturating_sub(self.detail.1));
        self.scroll = (i32::from(self.scroll) + delta).clamp(0, max) as u16;
    }

    pub fn draw(&mut self, frame: &mut Frame, images: &mut Images) {
        images.begin_frame();
        let [top, body, help, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        self.draw_query(frame, top);

        // The list above the note rather than beside it: full width reads better
        // than two narrow columns, and a note is mostly what you look at.
        let [list_area, detail_area] =
            Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(body);
        self.draw_list(frame, list_area);
        let panel = Panel::new()
            .borders(Borders::TOP)
            .border_style(Style::new().dim());
        let inner = panel.inner(detail_area);
        frame.render_widget(panel, detail_area);
        self.draw_detail(frame, inner, images);

        let help_line = if self.typing {
            Line::from(" enter/esc done   ↑/↓ move   ctrl-u clear").dim()
        } else if matches!(self.prompt, Some(Prompt::Tags(_))) {
            Line::from(" enter apply   tab complete   esc cancel   ctrl-u clear").dim()
        } else {
            // Scrolling, quitting, and T are one `?` away; this has to fit 80 columns.
            Line::from(
                " / search  e edit  d delete  s suspend  f flag  m mark  t tag  u undo  ? help",
            )
            .dim()
        };
        frame.render_widget(Paragraph::new(help_line), help);
        let bottom = match (&self.prompt, &self.status) {
            (Some(Prompt::ConfirmDelete { question, .. }), _) => Some(
                Paragraph::new(format!(" {question}"))
                    .bold()
                    .fg(Color::Yellow),
            ),
            (Some(Prompt::Tags(tags)), _) => Some(Paragraph::new(tags.line())),
            (None, Some(message)) => Some(Paragraph::new(format!(" {message}")).italic()),
            (None, None) => None,
        };
        if let Some(bottom) = bottom {
            frame.render_widget(bottom, status);
        }
        if self.help {
            overlay::keys(frame, "Browse keys", KEYS);
        }
    }

    fn draw_query(&self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![Span::raw(" / ").bold()];
        if self.query.is_empty() && !self.typing {
            spans.push(Span::raw("press / to search").dim());
        } else {
            spans.push(Span::raw(self.query.clone()).bold());
        }
        if self.typing {
            spans.push(Span::raw("▏"));
        }
        let count = match self.notes.len() {
            0 => String::new(),
            1 => "1 note ".to_string(),
            n => format!("{n} notes "),
        };
        let [left, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(count.len() as u16)])
                .areas(area);
        frame.render_widget(Paragraph::new(Line::from(spans)), left);
        frame.render_widget(
            Paragraph::new(count).dim().alignment(Alignment::Right),
            right,
        );
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        // One column goes to the highlight symbol, one to the leading space.
        let width = area.width.saturating_sub(2) as usize;
        let items: Vec<ListItem> = self
            .notes
            .iter()
            .map(|note| note_item(note, width))
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶");
        frame.render_stateful_widget(list, area, &mut self.list);
    }

    fn draw_detail(&mut self, frame: &mut Frame, area: Rect, images: &mut Images) {
        let inner = Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        let blocks = match self.selected() {
            Some(note) => detail_blocks(note),
            None if self.searched => vec![Block::Text(vec![Line::from("No notes match.").dim()])],
            None => vec![Block::Text(vec![
                Line::from("Type a search and press enter.").dim(),
            ])],
        };
        let total = blocks::draw(
            frame,
            inner,
            blocks,
            images,
            blocks::Options {
                align: Alignment::Left,
                vertical_center: false,
                scroll: self.scroll,
            },
        );
        self.detail = (total, inner.height);
        self.scroll = self.scroll.min(total.saturating_sub(inner.height));
    }
}

/// Sort field on the left; on the right the deck, with the flag and the mark in two
/// cells directly before its name, where the eye lands when scanning the deck column.
/// A suspended note is dimmed, the terminal's stand-in for the desktop's yellow row.
/// A narrow terminal loses the deck, not the marks.
fn note_item(note: &NoteView, width: usize) -> ListItem<'static> {
    let deck_width = if width >= 50 { 18 } else { 0 };
    // A space, the two mark cells, and another space before the deck when it is shown.
    let marks_width = if deck_width > 0 { 4 } else { 3 };
    let name_width = width.saturating_sub(marks_width + deck_width).max(1);
    let name = if note.sort_field.is_empty() {
        "(empty)"
    } else {
        &note.sort_field
    };
    let mut name = Span::raw(format!(" {:<name_width$}", truncate(name, name_width)));
    if note.suspended() {
        name = name.dim();
    }
    let flag = note.flag();
    let flag = if flag > 0 {
        Span::raw("⚑").fg(flag_color(flag))
    } else {
        Span::raw(" ")
    };
    let mark = if note.marked() {
        Span::raw("★").fg(Color::Yellow)
    } else {
        Span::raw(" ")
    };
    let mut spans = Vec::with_capacity(6);
    spans.push(name);
    if deck_width > 0 {
        // Right-align the deck by hand so the marks can carry their own colours.
        let deck = note.deck.rsplit("::").next().unwrap_or(&note.deck);
        let deck = truncate(deck, deck_width);
        let pad = deck_width + 1 - deck.chars().count();
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend([flag, mark, Span::raw(" "), Span::raw(deck).dim()]);
    } else {
        spans.extend([Span::raw(" "), flag, mark]);
    }
    ListItem::new(Line::from(spans))
}

/// Notetype, deck, flag, mark, and tags on top, then every field under its name, then
/// the cards. Fields render like cards do, so formatting and images show, but without
/// the notetype's stylesheet, which targets card templates rather than field content.
fn detail_blocks(note: &NoteView) -> Vec<Block> {
    let sheet = Stylesheet::parse("");
    let heading = |text: &str| {
        Line::from(Span::styled(
            text.to_string(),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
    };
    let mut header = vec![
        Span::raw(note.notetype.clone()).bold(),
        Span::raw("  "),
        Span::raw(note.deck.clone()).dim(),
    ];
    let flag = note.flag();
    if flag > 0 {
        header.push(Span::raw("  "));
        header.push(Span::raw(format!("⚑ {}", flag_name(flag))).fg(flag_color(flag)));
    }
    if note.marked() {
        header.push(Span::raw("  "));
        header.push(Span::raw("★ marked").fg(Color::Yellow));
    }
    if note.suspended() {
        header.push(Span::raw("  "));
        header.push(Span::raw("suspended").dim());
    }
    if !note.tags.is_empty() {
        header.push(Span::raw("  "));
        header.push(Span::raw(note.tags.join(" ")).fg(Color::Yellow));
    }
    let mut blocks = vec![Block::Text(vec![Line::from(header)])];
    for (name, html) in &note.fields.0 {
        blocks.push(Block::Text(vec![Line::from(""), heading(name)]));
        let body = html_to_blocks(html, &sheet);
        if body.is_empty() {
            blocks.push(Block::Text(vec![Line::from("(empty)").dim()]));
        } else {
            blocks.extend(body);
        }
    }
    blocks.push(Block::Text(vec![Line::from(""), heading("Cards")]));
    for card in &note.cards {
        let mut spans = vec![Span::raw(format!("{}  ", card.template)).bold()];
        for stat in card.stats() {
            spans.push(Span::raw(format!("{stat}  ")).dim());
        }
        if card.deck != note.deck {
            spans.push(Span::raw(card.deck.clone()).dim());
        }
        blocks.push(Block::Text(vec![Line::from(spans)]));
    }
    blocks
}

/// The search for every note in a deck and its subdecks, quoted when the name needs it.
pub fn deck_query(deck: &str) -> String {
    let special = |c: char| c.is_whitespace() || matches!(c, '"' | '(' | ')');
    if deck.chars().any(special) || deck.starts_with('-') {
        format!("\"deck:{}\"", deck.replace('"', "\\\""))
    } else {
        format!("deck:{deck}")
    }
}

/// Deletes the note with its cards and lists the results again with the next note
/// selected. Undoable with `u` while yaac runs; afterwards the deletion syncs.
pub fn delete(session: &mut Session, browser: &mut Browser, nid: NoteId) -> Result<()> {
    let removed = session.col.remove_notes(&[nid]).ctx("deleting note")?;
    rerun(session, browser)?;
    browser.set_status(format!(
        "deleted the note and its {} card(s); u undoes",
        removed.output
    ));
    Ok(())
}

/// Runs the query again unchanged, after changes that may have taken notes out of
/// it. The selection stays on its note, or moves to the one that took its place.
pub fn rerun(session: &mut Session, browser: &mut Browser) -> Result<()> {
    let index = browser.list.selected().unwrap_or(0);
    let selected = browser.selected().map(|note| note.id);
    search(session, browser)?;
    if browser.selected().map(|note| note.id) != selected {
        browser.select_nearest(index);
    }
    Ok(())
}

/// Puts up the tag prompt, completing from every tag in the collection when adding
/// and from the note's own when removing.
pub fn prompt_tags(
    session: &mut Session,
    browser: &mut Browser,
    nid: NoteId,
    mode: TagMode,
) -> Result<()> {
    let candidates = match mode {
        TagMode::Add => {
            let mut tags: Vec<String> = session
                .col
                .storage
                .all_tags()
                .ctx("reading tags")?
                .into_iter()
                .map(|tag| tag.name)
                .collect();
            tags.sort_by_key(|tag| tag.to_lowercase());
            tags
        }
        TagMode::Remove => browser
            .note(nid)
            .map(|note| note.tags.clone())
            .unwrap_or_default(),
    };
    browser.open_tag_prompt(nid, mode, candidates);
    Ok(())
}

/// Adds or removes the tags and says how many of them actually changed the note,
/// since Anki quietly skips tags it already has or never had.
pub fn apply_tags(
    session: &mut Session,
    browser: &mut Browser,
    nid: NoteId,
    mode: TagMode,
    tags: &[String],
) -> Result<()> {
    let Some(note) = browser.note(nid) else {
        return Ok(());
    };
    let before = note.tags.len();
    let joined = tags.join(" ");
    match mode {
        TagMode::Add => session
            .col
            .add_tags_to_notes(&[nid], &joined)
            .ctx("adding tags")?,
        TagMode::Remove => session
            .col
            .remove_tags_from_notes(&[nid], &joined)
            .ctx("removing tags")?,
    };
    refresh_note(session, browser, nid)?;
    let after = browser.note(nid).map_or(before, |note| note.tags.len());
    browser.set_status(match mode {
        TagMode::Add if after > before => format!("added {} tag(s)", after - before),
        TagMode::Add => "already tagged".to_string(),
        TagMode::Remove if before > after => format!("removed {} tag(s)", before - after),
        TagMode::Remove => "no such tag".to_string(),
    });
    Ok(())
}

/// Swaps in a fresh view of the note after something about it changed.
fn refresh_note(session: &mut Session, browser: &mut Browser, nid: NoteId) -> Result<()> {
    if let Some(view) = notes::views(&mut session.col, &[nid])?.pop() {
        browser.replace_note(view);
    }
    Ok(())
}

fn card_ids(note: &NoteView) -> Vec<CardId> {
    note.cards.iter().map(|card| CardId(card.id)).collect()
}

/// Suspends every card of the note, or unsuspends them all once none is left to
/// suspend: the desktop browser's toggle, applied to the whole note.
pub fn toggle_suspend(session: &mut Session, browser: &mut Browser, nid: NoteId) -> Result<()> {
    let Some(note) = browser.note(nid) else {
        return Ok(());
    };
    let cids = card_ids(note);
    let status = if note.suspended() {
        session
            .col
            .unbury_or_unsuspend_cards(&cids)
            .ctx("unsuspending cards")?;
        format!("unsuspended {} card(s)", cids.len())
    } else {
        session
            .col
            .bury_or_suspend_cards(&cids, BuryOrSuspendMode::Suspend)
            .ctx("suspending cards")?;
        format!("suspended {} card(s)", cids.len())
    };
    refresh_note(session, browser, nid)?;
    browser.set_status(status);
    Ok(())
}

/// Steps the flag on every card of the note through Anki's seven colours and back
/// to none, starting from the first flag any card has.
pub fn cycle_flag(session: &mut Session, browser: &mut Browser, nid: NoteId) -> Result<()> {
    let Some(note) = browser.note(nid) else {
        return Ok(());
    };
    let cids = card_ids(note);
    let next = notes::next_flag(note.flag());
    session.col.set_card_flag(&cids, next).ctx("setting flag")?;
    refresh_note(session, browser, nid)?;
    browser.set_status(if next == 0 {
        "flag removed".to_string()
    } else {
        format!("flagged {}", flag_name(next))
    });
    Ok(())
}

pub fn toggle_mark(session: &mut Session, browser: &mut Browser, nid: NoteId) -> Result<()> {
    let Some(note) = browser.note(nid) else {
        return Ok(());
    };
    let marked = !note.marked();
    notes::set_marked(&mut session.col, nid, marked)?;
    refresh_note(session, browser, nid)?;
    browser.set_status(if marked { "marked" } else { "unmarked" });
    Ok(())
}

/// Runs the browser's query, which happens on every keystroke. An empty query lists
/// nothing, because the box starts empty and loading the whole collection then would
/// be wasted work. A search the collection rejects becomes a status message rather
/// than an error, since a half-typed query is often not valid yet.
pub fn search(session: &mut Session, browser: &mut Browser) -> Result<()> {
    if browser.query().trim().is_empty() {
        browser.clear_results();
        browser.clear_status();
        return Ok(());
    }
    let by_sort_field = SortMode::Builtin {
        column: Column::SortField,
        reverse: false,
    };
    match session.col.search_notes(browser.query(), by_sort_field) {
        Ok(nids) => {
            let views = notes::views(&mut session.col, &nids)?;
            browser.clear_status();
            browser.set_notes(views);
        }
        Err(err) => browser.set_status(format!("search failed: {:#}", anki_error(err))),
    }
    Ok(())
}

/// Runs the screen until the user leaves. Searching, editing, deleting, and undo
/// happen here because they need the session, and editing needs the terminal too.
pub fn run(
    terminal: &mut Terminal,
    session: &mut Session,
    browser: &mut Browser,
    images: &mut Images,
) -> Result<Exit> {
    search(session, browser)?;
    loop {
        terminal.draw(|frame| browser.draw(frame, images))?;
        images.end_frame();
        let Some(key) = next_key(Duration::from_millis(250))? else {
            continue;
        };
        match browser.handle(key) {
            BrowseAction::Continue => {}
            BrowseAction::Redraw => images.clear(),
            BrowseAction::Refresh => images.refresh(),
            BrowseAction::Search => search(session, browser)?,
            BrowseAction::Edit(nid) => {
                let editor = Editor::from_env();
                let outcome =
                    terminal.suspend(|| editor::edit_note(&mut session.col, nid, &editor))?;
                images.clear();
                match outcome {
                    Ok(outcome) => {
                        if outcome == Outcome::Saved {
                            refresh_note(session, browser, nid)?;
                        }
                        browser.set_status(outcome.message());
                    }
                    Err(err) => browser.set_status(format!("edit failed: {err:#}")),
                }
            }
            BrowseAction::Delete(nid) => delete(session, browser, nid)?,
            BrowseAction::Suspend(nid) => toggle_suspend(session, browser, nid)?,
            BrowseAction::Flag(nid) => cycle_flag(session, browser, nid)?,
            BrowseAction::Mark(nid) => toggle_mark(session, browser, nid)?,
            BrowseAction::TagPrompt(nid, mode) => prompt_tags(session, browser, nid, mode)?,
            BrowseAction::Tags(nid, mode, tags) => apply_tags(session, browser, nid, mode, &tags)?,
            BrowseAction::Rerun => rerun(session, browser)?,
            BrowseAction::Undo => match session.col.undo() {
                Ok(_) => {
                    search(session, browser)?;
                    browser.set_status("undone");
                }
                Err(AnkiError::UndoEmpty) => browser.set_status("nothing to undo"),
                Err(err) => browser.set_status(format!("undo failed: {:#}", anki_error(err))),
            },
            BrowseAction::Back => return Ok(Exit::Back),
            BrowseAction::Quit => return Ok(Exit::Quit),
        }
    }
}
