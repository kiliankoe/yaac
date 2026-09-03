//! Deck picker shown by `review` when no deck was named.

use std::time::Duration;

use anki::decks::DeckId;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::decks::DeckRow;
use crate::tui::{Terminal, next_key};

pub fn pick(terminal: &mut Terminal, rows: &[DeckRow]) -> Result<Option<DeckId>> {
    let mut state = ListState::default();
    // Start on the first deck with something due, which is what most sessions want.
    state.select(Some(rows.iter().position(|row| row.due() > 0).unwrap_or(0)));
    loop {
        terminal.draw(|frame| draw(frame, rows, &mut state))?;
        let Some(key) = next_key(Duration::from_millis(250))? else {
            continue;
        };
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            KeyCode::Down | KeyCode::Char('j') => state.select_next(),
            KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
            KeyCode::Home | KeyCode::Char('g') => state.select_first(),
            KeyCode::End | KeyCode::Char('G') => state.select_last(),
            KeyCode::Enter => {
                if let Some(row) = state.selected().and_then(|i| rows.get(i)) {
                    return Ok(Some(DeckId(row.id)));
                }
            }
            _ => {}
        }
    }
}

pub fn draw(frame: &mut Frame, rows: &[DeckRow], state: &mut ListState) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let name_width = body.width.saturating_sub(24).max(10) as usize;
    let header = Line::from(vec![
        Span::raw(format!(" {:<width$}", "Choose a deck", width = name_width)),
        Span::raw(format!("{:>6} {:>6} {:>7}", "new", "learn", "review")).dim(),
    ]);
    frame.render_widget(Paragraph::new(header).bold(), top);

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.level.saturating_sub(1) as usize);
            let name = format!("{indent}{}", row.short_name());
            let name = if name.chars().count() > name_width {
                name.chars().take(name_width - 1).chain(['…']).collect()
            } else {
                name
            };
            let quiet = row.due() == 0;
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
            if quiet {
                line = line.dim();
            }
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶");
    frame.render_stateful_widget(list, body, state);

    frame.render_widget(
        Paragraph::new(" enter review   j/k move   q quit").dim(),
        bottom,
    );
}
