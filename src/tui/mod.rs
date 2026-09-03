//! Terminal setup shared by the interactive screens.

pub mod decks;
pub mod review;

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;

/// Raw mode and the alternate screen, undone on drop so errors and early returns leave
/// the shell usable.
pub struct Terminal(DefaultTerminal);

impl Terminal {
    pub fn open() -> Self {
        Self(ratatui::init())
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
