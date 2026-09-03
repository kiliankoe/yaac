//! Deck picker shown by `review`: lists decks with today's counts, filters on `/`,
//! syncs on `s`, and stays alive across review sessions so Esc from a review lands
//! back here with fresh counts.

use std::time::Duration;

use anki::decks::DeckId;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::decks::{self, DeckRow};
use crate::session::Session;
use crate::sync::{self, Auth, NormalOutcome};
use crate::tui::{Terminal, next_key};

pub enum Choice {
    Deck(DeckId),
    Quit,
}

/// What a key press asks the picker loop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerAction {
    Continue,
    Select(DeckId),
    Sync,
    Quit,
}

pub struct Picker {
    rows: Vec<DeckRow>,
    filter: String,
    searching: bool,
    list: ListState,
    status: Option<String>,
}

impl Picker {
    pub fn new(rows: Vec<DeckRow>) -> Self {
        let mut picker = Self {
            rows,
            filter: String::new(),
            searching: false,
            list: ListState::default(),
            status: None,
        };
        // Start on the first deck with something due, which is what most sessions want.
        let first_due = picker
            .rows
            .iter()
            .position(|row| row.due() > 0)
            .unwrap_or(0);
        picker.list.select(Some(first_due));
        picker
    }

    /// Replaces the rows (after a sync or a review) while keeping the selected deck.
    pub fn set_rows(&mut self, rows: Vec<DeckRow>) {
        let selected = self.selected().map(|row| row.id);
        self.rows = rows;
        let index = selected
            .and_then(|id| self.visible().iter().position(|row| row.id == id))
            .unwrap_or(0);
        self.list.select(Some(index));
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    /// Rows matching the filter, by case-insensitive substring of the full name.
    pub fn visible(&self) -> Vec<&DeckRow> {
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .filter(|row| needle.is_empty() || row.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected(&self) -> Option<&DeckRow> {
        let visible = self.visible();
        self.list
            .selected()
            .and_then(|index| visible.get(index).copied())
    }

    pub fn handle(&mut self, key: KeyEvent) -> PickerAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return PickerAction::Quit;
        }
        match key.code {
            KeyCode::Down => self.list.select_next(),
            KeyCode::Up => self.list.select_previous(),
            KeyCode::Enter => {
                self.searching = false;
                if let Some(row) = self.selected() {
                    return PickerAction::Select(DeckId(row.id));
                }
            }
            // Esc dismisses an active filter first, whether or not it is being typed;
            // only an unfiltered list quits.
            KeyCode::Esc if self.searching || !self.filter.is_empty() => {
                self.filter.clear();
                self.searching = false;
                self.list.select_first();
            }
            code if self.searching => match code {
                KeyCode::Backspace => {
                    if self.filter.pop().is_none() {
                        self.searching = false;
                    }
                    self.list.select_first();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.list.select_first();
                }
                _ => {}
            },
            KeyCode::Char('q') => return PickerAction::Quit,
            KeyCode::Char('/') if !self.filter.is_empty() => self.searching = true,
            KeyCode::Char('/') => {
                self.searching = true;
                self.status = None;
            }
            KeyCode::Char('s') => return PickerAction::Sync,
            KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Char('g') | KeyCode::Home => self.list.select_first(),
            KeyCode::Char('G') | KeyCode::End => self.list.select_last(),
            _ => {}
        }
        PickerAction::Continue
    }

    /// A normal plus media sync with a one-line outcome for the status line. Full syncs
    /// are deliberately left to the CLI, where the direction is confirmed.
    pub fn sync(&mut self, session: &mut Session, auth: Option<Auth>) -> String {
        let Some(mut auth) = auth else {
            return "not logged in; run `yaac login` first".to_string();
        };
        let report = match sync::normal(session, &mut auth) {
            Ok(report) => report,
            Err(err) => return format!("sync failed: {err:#}"),
        };
        let changed = match report.outcome {
            NormalOutcome::Done { changed } => changed,
            NormalOutcome::FullSyncRequired { .. } => {
                return "a full sync is required; run `yaac sync` to choose a direction"
                    .to_string();
            }
        };
        if let Err(err) = sync::media(session, &auth) {
            return format!("collection synced, media sync failed: {err:#}");
        }
        if let Err(err) = sync::save_auth(&auth) {
            return format!("synced, but could not save credentials: {err:#}");
        }
        if changed {
            "synced"
        } else {
            "already up to date"
        }
        .to_string()
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let [top, body, help, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let name_width = body.width.saturating_sub(24).max(10) as usize;
        let header = Line::from(vec![
            Span::raw(format!(" {:<width$}", "Choose a deck", width = name_width)),
            Span::raw(format!("{:>6} {:>6} {:>7}", "new", "learn", "review")).dim(),
        ]);
        frame.render_widget(Paragraph::new(header).bold(), top);

        let items: Vec<ListItem> = self
            .visible()
            .into_iter()
            .map(|row| deck_item(row, name_width))
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶");
        frame.render_stateful_widget(list, body, &mut self.list);

        let help_line = if self.searching {
            Line::from(vec![
                Span::raw(" /").bold(),
                Span::raw(self.filter.clone()).bold(),
                Span::raw("▏"),
                Span::raw("   enter review   esc clear").dim(),
            ])
        } else if self.filter.is_empty() {
            Line::from(" enter review   / search   s sync   j/k move   q quit").dim()
        } else {
            Line::from(vec![
                Span::raw(format!(" filter: {}   ", self.filter)),
                Span::raw("enter review   / edit   s sync   q quit").dim(),
            ])
        };
        frame.render_widget(Paragraph::new(help_line), help);
        if let Some(message) = &self.status {
            frame.render_widget(Paragraph::new(format!(" {message}")).italic(), status);
        }
    }
}

fn deck_item(row: &DeckRow, name_width: usize) -> ListItem<'static> {
    let indent = "  ".repeat(row.level.saturating_sub(1) as usize);
    let name = format!("{indent}{}", row.short_name());
    let name = if name.chars().count() > name_width {
        name.chars().take(name_width - 1).chain(['…']).collect()
    } else {
        name
    };
    let count = |n: u32, color: Color| {
        let span = Span::raw(format!("{n:>6}"));
        if n == 0 { span.dim() } else { span.fg(color) }
    };
    let mut line = Line::from(vec![
        Span::raw(format!(" {name:<name_width$}")),
        count(row.new, Color::Blue),
        Span::raw(" "),
        count(row.learn, Color::Red),
        Span::raw(" "),
        Span::raw(format!("{:>7}", row.review)).fg(if row.review == 0 {
            Color::DarkGray
        } else {
            Color::Green
        }),
    ]);
    if row.due() == 0 {
        line = line.dim();
    }
    ListItem::new(line)
}

/// Runs the picker until a deck is chosen or the user quits. Syncing happens here
/// because it needs the terminal for a "syncing" frame and the session for the work.
pub fn pick(terminal: &mut Terminal, session: &mut Session, picker: &mut Picker) -> Result<Choice> {
    loop {
        terminal.draw(|frame| picker.draw(frame))?;
        let Some(key) = next_key(Duration::from_millis(250))? else {
            continue;
        };
        match picker.handle(key) {
            PickerAction::Continue => {}
            PickerAction::Select(deck) => return Ok(Choice::Deck(deck)),
            PickerAction::Quit => return Ok(Choice::Quit),
            PickerAction::Sync => {
                picker.set_status("syncing…");
                terminal.draw(|frame| picker.draw(frame))?;
                let auth = match sync::load_auth() {
                    Ok(auth) => auth,
                    Err(err) => {
                        picker.set_status(format!("cannot read credentials: {err:#}"));
                        continue;
                    }
                };
                let outcome = picker.sync(session, auth);
                picker.set_status(outcome);
                picker.set_rows(decks::rows(&mut session.col)?);
            }
        }
    }
}
