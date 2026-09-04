//! The browse screen: a search box on top, matching notes on one side, the selected
//! note's fields, tags, and cards on the other. `e` opens the note in `$EDITOR`, `d`
//! deletes it after asking.

use std::time::Duration;

use anki::browser_table::Column;
use anki::error::AnkiError;
use anki::notes::NoteId;
use anki::search::SortMode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as Panel, Borders, List, ListItem, ListState, Paragraph};

use crate::editor::{self, Editor, Outcome};
use crate::notes::{self, NoteView, truncate};
use crate::render::{Block, Stylesheet, html_to_blocks};
use crate::session::{AnkiResultExt, Session, anki_error};
use crate::tui::images::Images;
use crate::tui::{Terminal, blocks, is_ctrl_c, next_key, overlay};

/// Side-by-side panes need this much width; narrower terminals stack them.
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 100;

const KEYS: &[(&str, &str)] = &[
    ("/", "type a search; enter or esc leaves the box"),
    ("ctrl-u", "clear the search while typing"),
    ("j/k, ↑/↓", "move through the notes"),
    ("g/G", "first and last note"),
    ("ctrl-d/u, page down/up", "scroll the note"),
    ("e", "edit the note in $EDITOR"),
    ("d", "delete the note, after confirming"),
    ("u", "undo the last edit or deletion"),
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
    /// `d` was pressed on this note and the status line asks for a `y`.
    confirming: Option<NoteId>,
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
            confirming: None,
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

    pub fn handle(&mut self, key: KeyEvent) -> BrowseAction {
        if is_ctrl_c(key) {
            return BrowseAction::Quit;
        }
        // The overlay swallows the key that closes it.
        if self.help {
            self.help = false;
            return BrowseAction::Refresh;
        }
        if let Some(nid) = self.confirming.take() {
            self.status = None;
            if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                return BrowseAction::Delete(nid);
            }
            return BrowseAction::Continue;
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
            KeyCode::Char('e') => {
                if let Some(note) = self.selected() {
                    return BrowseAction::Edit(NoteId(note.id));
                }
            }
            KeyCode::Char('r') => return BrowseAction::Redraw,
            KeyCode::Char('d') if ctrl => self.scroll_by(self.half_page()),
            KeyCode::Char('u') if ctrl => self.scroll_by(-self.half_page()),
            KeyCode::Char('d') => {
                if let Some(note) = self.selected() {
                    let prompt = format!(
                        "delete \"{}\" and its {} card(s)? y confirms, any other key cancels",
                        truncate(&note.sort_field, 40),
                        note.cards.len()
                    );
                    self.confirming = Some(NoteId(note.id));
                    self.status = Some(prompt);
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

        let side_by_side = body.width >= SIDE_BY_SIDE_MIN_WIDTH;
        let split = [Constraint::Percentage(40), Constraint::Percentage(60)];
        let [list_area, detail_area] = if side_by_side {
            Layout::horizontal(split).areas(body)
        } else {
            Layout::vertical(split).areas(body)
        };
        self.draw_list(frame, list_area);
        let panel = Panel::new()
            .borders(if side_by_side {
                Borders::LEFT
            } else {
                Borders::TOP
            })
            .border_style(Style::new().dim());
        let inner = panel.inner(detail_area);
        frame.render_widget(panel, detail_area);
        self.draw_detail(frame, inner, images);

        let help_line = if self.typing {
            Line::from(" enter/esc done   ↑/↓ move   ctrl-u clear").dim()
        } else {
            Line::from(" / search   e edit   d delete   u undo   ctrl-d/u scroll   q quit   ? help")
                .dim()
        };
        frame.render_widget(Paragraph::new(help_line), help);
        if let Some(message) = &self.status {
            let message = Paragraph::new(format!(" {message}"));
            let message = if self.confirming.is_some() {
                message.bold().fg(Color::Yellow)
            } else {
                message.italic()
            };
            frame.render_widget(message, status);
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

fn note_item(note: &NoteView, width: usize) -> ListItem<'static> {
    let deck_width = if width >= 50 { 18 } else { 0 };
    let name_width = width.saturating_sub(deck_width).max(1);
    let name = if note.sort_field.is_empty() {
        "(empty)"
    } else {
        &note.sort_field
    };
    let mut spans = vec![Span::raw(format!(
        " {:<name_width$}",
        truncate(name, name_width)
    ))];
    if deck_width > 0 {
        let deck = note.deck.rsplit("::").next().unwrap_or(&note.deck);
        spans.push(Span::raw(format!("{:>deck_width$}", truncate(deck, deck_width))).dim());
    }
    ListItem::new(Line::from(spans))
}

/// Notetype, deck, and tags on top, then every field under its name, then the cards.
/// Fields render like cards do, so formatting and images show, but without the
/// notetype's stylesheet, which targets card templates rather than field content.
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
    let index = browser.list.selected().unwrap_or(0);
    let removed = session.col.remove_notes(&[nid]).ctx("deleting note")?;
    search(session, browser)?;
    browser.select_nearest(index);
    browser.set_status(format!(
        "deleted the note and its {} card(s); u undoes",
        removed.output
    ));
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
                            if let Some(view) = notes::views(&mut session.col, &[nid])?.pop() {
                                browser.replace_note(view);
                            }
                        }
                        browser.set_status(outcome.message());
                    }
                    Err(err) => browser.set_status(format!("edit failed: {err:#}")),
                }
            }
            BrowseAction::Delete(nid) => delete(session, browser, nid)?,
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
