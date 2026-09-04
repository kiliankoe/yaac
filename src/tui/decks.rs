//! Deck picker shown by `review`: lists decks with today's counts, filters on `/`,
//! syncs on `s`, adds a note to the selected deck on `a`, and stays alive across
//! review sessions so Esc from a review lands back here with fresh counts.

use std::time::{Duration, Instant};

use anki::decks::DeckId;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::decks::{self, DeckRow};
use crate::editor::{self, Editor};
use crate::notes;
use crate::session::{AnkiResultExt, Session};
use crate::sync::{self, Auth, NormalOutcome};
use crate::tui::{Terminal, is_ctrl_c, next_key, overlay};

const KEYS: &[(&str, &str)] = &[
    ("enter", "review the selected deck"),
    ("a", "add a note to the selected deck"),
    ("A", "add another note of the notetype used last"),
    ("b", "browse the selected deck's notes"),
    ("s", "sync the collection and media"),
    ("/", "filter decks by name"),
    ("esc", "clear the filter"),
    ("j/k, ↑/↓", "move"),
    ("g/G", "first and last deck"),
    ("q", "quit"),
];

/// How long a status message stays on screen before it clears itself, so a stale
/// "synced" does not sit under the deck list for the rest of the session.
const STATUS_TIMEOUT: Duration = Duration::from_secs(4);

pub enum Choice {
    Deck(DeckId),
    /// Open the browse screen on the deck's notes.
    Browse(DeckId),
    Quit,
}

/// What a key press asks the picker loop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerAction {
    Continue,
    Select(DeckId),
    Sync,
    /// Add a note of the notetype to the deck, in the editor.
    Add {
        deck: DeckId,
        notetype: String,
    },
    Browse(DeckId),
    Quit,
}

pub struct Picker {
    rows: Vec<DeckRow>,
    filter: String,
    searching: bool,
    list: ListState,
    /// The status line message and when it was set; see [`STATUS_TIMEOUT`].
    status: Option<(String, Instant)>,
    /// Notetype names offered by `a`, and the one used last (or the config's
    /// default), which the chooser starts on and `A` takes without asking.
    notetypes: Vec<String>,
    last_notetype: Option<usize>,
    /// The chooser opened by `a`: the deck the note goes to and the highlighted
    /// notetype.
    chooser: Option<(DeckId, ListState)>,
    /// The `?` overlay is up.
    help: bool,
}

impl Picker {
    pub fn new(rows: Vec<DeckRow>, notetypes: Vec<String>) -> Self {
        let mut picker = Self {
            rows,
            filter: String::new(),
            searching: false,
            list: ListState::default(),
            status: None,
            notetypes,
            last_notetype: None,
            chooser: None,
            help: false,
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
        self.status = Some((status.into(), Instant::now()));
    }

    /// Drops a status message once it has been up long enough to read. The picker loop
    /// calls this before every draw, which is often enough for the message to vanish on
    /// its own without a key press.
    pub fn expire_status(&mut self, now: Instant) {
        let expired = self
            .status
            .as_ref()
            .is_some_and(|(_, set_at)| now.duration_since(*set_at) >= STATUS_TIMEOUT);
        if expired {
            self.status = None;
        }
    }

    /// The config's default notetype counts as used last until a note is added.
    pub fn set_default_notetype(&mut self, name: &str) {
        if let Some(index) = self.notetypes.iter().position(|n| n == name) {
            self.last_notetype = Some(index);
        }
    }

    pub fn choosing_notetype(&self) -> bool {
        self.chooser.is_some()
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
        if is_ctrl_c(key) {
            return PickerAction::Quit;
        }
        // The overlay swallows the key that closes it.
        if self.help {
            self.help = false;
            return PickerAction::Continue;
        }
        if self.chooser.is_some() {
            return self.handle_chooser(key);
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
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('b') => {
                if let Some(row) = self.selected() {
                    return PickerAction::Browse(DeckId(row.id));
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let Some(deck) = self.selected().map(|row| DeckId(row.id)) else {
                    return PickerAction::Continue;
                };
                if self.notetypes.is_empty() {
                    self.set_status("no notetypes to add a note with");
                    return PickerAction::Continue;
                }
                if key.code == KeyCode::Char('A') {
                    if let Some(notetype) = self.last_notetype.and_then(|i| self.notetypes.get(i)) {
                        return PickerAction::Add {
                            deck,
                            notetype: notetype.clone(),
                        };
                    }
                }
                let list =
                    ListState::default().with_selected(Some(self.last_notetype.unwrap_or(0)));
                self.chooser = Some((deck, list));
            }
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

    fn handle_chooser(&mut self, key: KeyEvent) -> PickerAction {
        let last = self.notetypes.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.chooser = None,
            KeyCode::Enter => {
                if let Some((deck, list)) = self.chooser.take() {
                    if let Some(index) = list.selected().filter(|&i| i < self.notetypes.len()) {
                        self.last_notetype = Some(index);
                        return PickerAction::Add {
                            deck,
                            notetype: self.notetypes[index].clone(),
                        };
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some((_, list)) = &mut self.chooser {
                    list.select(Some((list.selected().unwrap_or(0) + 1).min(last)));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some((_, list)) = &mut self.chooser {
                    list.select(Some(list.selected().unwrap_or(0).saturating_sub(1)));
                }
            }
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
            Line::from(" enter review   / search   s sync   a/A add   b browse   q quit   ? help")
                .dim()
        } else {
            Line::from(vec![
                Span::raw(format!(" filter: {}   ", self.filter)),
                Span::raw("enter review   / edit   s sync   a/A add   b browse   q quit   ? help")
                    .dim(),
            ])
        };
        frame.render_widget(Paragraph::new(help_line), help);
        if let Some((message, _)) = &self.status {
            frame.render_widget(Paragraph::new(format!(" {message}")).italic(), status);
        }
        if let Some((deck, list)) = &mut self.chooser {
            let deck_name = self
                .rows
                .iter()
                .find(|row| row.id == deck.0)
                .map_or("deck", |row| row.name.as_str());
            let width = self
                .notetypes
                .iter()
                .map(|name| name.chars().count() + 2)
                .max()
                .unwrap_or(0);
            let inner = overlay::boxed(
                frame,
                &format!("Add to {deck_name}"),
                "enter choose   esc cancel",
                width as u16,
                self.notetypes.len() as u16,
            );
            let items: Vec<ListItem> = self
                .notetypes
                .iter()
                .map(|name| ListItem::new(format!(" {name}")))
                .collect();
            let chooser = List::new(items)
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol("▶");
            frame.render_stateful_widget(chooser, inner, list);
        }
        if self.help {
            overlay::keys(frame, "Deck keys", KEYS);
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

/// Runs the picker until a deck is chosen or the user quits. Syncing and adding happen
/// here because they need the session, and the terminal for a "syncing" frame or for
/// the editor.
pub fn pick(terminal: &mut Terminal, session: &mut Session, picker: &mut Picker) -> Result<Choice> {
    loop {
        picker.expire_status(Instant::now());
        terminal.draw(|frame| picker.draw(frame))?;
        let Some(key) = next_key(Duration::from_millis(250))? else {
            continue;
        };
        match picker.handle(key) {
            PickerAction::Continue => {}
            PickerAction::Select(deck) => return Ok(Choice::Deck(deck)),
            PickerAction::Browse(deck) => return Ok(Choice::Browse(deck)),
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
            PickerAction::Add { deck, notetype } => {
                let Some(notetype) = session
                    .col
                    .get_notetype_by_name(&notetype)
                    .ctx("looking up notetype")?
                else {
                    picker.set_status(format!("notetype {notetype:?} no longer exists"));
                    continue;
                };
                let editor = Editor::from_env();
                let outcome = terminal
                    .suspend(|| editor::add_note(&mut session.col, deck, &notetype, &editor))?;
                match outcome {
                    Ok(Some(_)) => {
                        let deck_name = notes::deck_name(&mut session.col, deck)?;
                        picker.set_status(format!("added a {} note to {deck_name}", notetype.name));
                        picker.set_rows(decks::rows(&mut session.col)?);
                    }
                    Ok(None) => picker.set_status("aborted"),
                    Err(err) => picker.set_status(format!("add failed: {err:#}")),
                }
            }
        }
    }
}
