//! Terminal setup shared by the interactive screens.

pub mod blocks;
pub mod browse;
pub mod decks;
pub mod images;
pub mod kitty;
pub mod overlay;
pub mod review;

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::DefaultTerminal;
use ratatui::style::Color;

/// Raw mode and the alternate screen, undone on drop so errors and early returns leave
/// the shell usable.
pub struct Terminal(DefaultTerminal);

impl Terminal {
    pub fn open() -> Self {
        Self(ratatui::init())
    }

    /// Hands the terminal to something else, such as an editor, while `f` runs: raw
    /// mode and the alternate screen are left and re-entered around it, and the next
    /// draw repaints everything.
    pub fn suspend<T>(&mut self, f: impl FnOnce() -> T) -> Result<T> {
        ratatui::restore();
        let result = f();
        enable_raw_mode().context("re-entering raw mode")?;
        execute!(std::io::stdout(), EnterAlternateScreen)
            .context("re-entering the alternate screen")?;
        self.0.clear().context("clearing the screen")?;
        Ok(result)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

impl Deref for Terminal {
    type Target = DefaultTerminal;
    fn deref(&self) -> &DefaultTerminal {
        &self.0
    }
}

impl DerefMut for Terminal {
    fn deref_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.0
    }
}

/// The next key press within `timeout`, or None so the caller can redraw (a clock,
/// a resize). Key releases and repeats are ignored for terminals that report them.
pub fn next_key(timeout: Duration) -> Result<Option<KeyEvent>> {
    if !event::poll(timeout).context("waiting for input")? {
        return Ok(None);
    }
    match event::read().context("reading input")? {
        Event::Key(key) if key.kind == KeyEventKind::Press => Ok(Some(key)),
        _ => Ok(None),
    }
}

/// Ctrl-c quits every screen, whatever else is going on.
pub fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Terminal colours for Anki's seven flags; 0 is no flag.
pub fn flag_color(flag: u32) -> Color {
    match flag {
        1 => Color::Red,
        2 => Color::Rgb(255, 140, 0),
        3 => Color::Green,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Rgb(160, 32, 240),
        _ => Color::Reset,
    }
}
