//! The review screen: state on top, the card centered, answers along the bottom.

use std::time::Duration;

use anki::scheduler::answering::Rating;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::editor::Editor;
use crate::notes::flag_name;
use crate::render::{Block, Stylesheet, html_to_blocks};
use crate::review::{Kind, Reviewer};
use crate::tui::images::Images;
use crate::tui::{Terminal, blocks, is_ctrl_c, next_key, overlay};

pub const AGAIN: Color = Color::Red;
pub const HARD: Color = Color::Yellow;
pub const GOOD: Color = Color::Green;
pub const EASY: Color = Color::Blue;

const KEYS: &[(&str, &str)] = &[
    ("space, enter", "show the answer, or the question again"),
    ("1 2 3 4", "Again, Hard, Good, Easy"),
    ("u", "undo the last answer or change"),
    ("s", "suspend the card"),
    ("b", "bury the card until tomorrow"),
    ("f", "cycle the card's flag colour"),
    ("m", "mark or unmark the note"),
    ("e", "edit the note in $EDITOR"),
    ("r", "re-send and redraw the images"),
    ("esc", "back to the deck list"),
    ("q", "quit"),
];

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    /// Back to the deck list.
    Back,
    /// Leave the TUI altogether.
    Quit,
}

/// Runs the review loop until the user leaves; returns whether they went back to the
/// deck list or quit.
pub fn run(
    terminal: &mut Terminal,
    reviewer: &mut Reviewer,
    images: &mut Images,
) -> Result<Action> {
    let mut status: Option<String> = None;
    let mut help = false;
    loop {
        terminal.draw(|frame| {
            draw(frame, reviewer, images, status.as_deref());
            if help {
                overlay::keys(frame, "Review keys", KEYS);
            }
        })?;
        images.end_frame();
        let Some(key) = next_key(Duration::from_millis(250))? else {
            continue;
        };
        // The overlay swallows the key that closes it, except ctrl-c.
        if help && !is_ctrl_c(key) {
            help = false;
            images.refresh();
            continue;
        }
        match key.code {
            KeyCode::Char('?') => {
                help = true;
                continue;
            }
            KeyCode::Char('r') => {
                images.clear();
                continue;
            }
            // Editing leaves the alternate screen, so it lives here rather than in
            // `handle`, which tests drive without a terminal.
            KeyCode::Char('e') => {
                let editor = Editor::from_env();
                let outcome = terminal.suspend(|| reviewer.edit(&editor))?;
                images.clear();
                status = match outcome {
                    Ok(Some(outcome)) => Some(outcome.message().to_string()),
                    Ok(None) => None,
                    Err(err) => Some(format!("edit failed: {err:#}")),
                };
                continue;
            }
            _ => status = None,
        }
        match handle(reviewer, key)? {
            Action::Continue => {}
            action => return Ok(action),
        }
    }
}

pub fn handle(reviewer: &mut Reviewer, key: KeyEvent) -> Result<Action> {
    let revealed = reviewer.current.as_ref().is_some_and(|c| c.revealed);
    match key.code {
        _ if is_ctrl_c(key) => return Ok(Action::Quit),
        KeyCode::Char('q') => return Ok(Action::Quit),
        KeyCode::Esc => return Ok(Action::Back),
        KeyCode::Char(' ') | KeyCode::Enter => reviewer.toggle_reveal(),
        KeyCode::Char('1') if revealed => reviewer.answer(Rating::Again)?,
        KeyCode::Char('2') if revealed => reviewer.answer(Rating::Hard)?,
        KeyCode::Char('3') if revealed => reviewer.answer(Rating::Good)?,
        KeyCode::Char('4') if revealed => reviewer.answer(Rating::Easy)?,
        KeyCode::Char('u') => {
            reviewer.undo()?;
        }
        KeyCode::Char('s') => reviewer.suspend()?,
        KeyCode::Char('b') => reviewer.bury()?,
        KeyCode::Char('f') => reviewer.cycle_flag()?,
        KeyCode::Char('m') => reviewer.toggle_mark()?,
        _ => {}
    }
    Ok(Action::Continue)
}

pub fn draw(frame: &mut Frame, reviewer: &Reviewer, images: &mut Images, status: Option<&str>) {
    images.begin_frame();
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    draw_status(frame, top, reviewer);
    draw_card(frame, body, reviewer, images);
    draw_actions(frame, bottom, reviewer, status);
}

fn draw_status(frame: &mut Frame, area: Rect, reviewer: &Reviewer) {
    let kind = reviewer.current.as_ref().map(|c| c.kind);
    let count = |label: &str, n: usize, color: Color, active: bool| {
        let span = Span::raw(format!("{label} {n}")).fg(color);
        if active {
            span.underlined().bold()
        } else {
            span
        }
    };
    let left = Line::from(vec![
        Span::raw(format!(" {}", reviewer.deck)).bold(),
        Span::raw("   "),
        count(
            "new",
            reviewer.counts.new,
            Color::Blue,
            kind == Some(Kind::New),
        ),
        Span::raw("  "),
        count(
            "learn",
            reviewer.counts.learning,
            Color::Red,
            kind == Some(Kind::Learning),
        ),
        Span::raw("  "),
        count(
            "review",
            reviewer.counts.review,
            Color::Green,
            kind == Some(Kind::Review),
        ),
    ]);
    let secs = reviewer.elapsed().as_secs();
    let right = Line::from(format!(
        "{} answered   {:02}:{:02} ",
        reviewer.answered,
        secs / 60,
        secs % 60
    ))
    .dim()
    .right_aligned();
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(26)]).areas(area);
    frame.render_widget(Paragraph::new(left), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

fn draw_card(frame: &mut Frame, area: Rect, reviewer: &Reviewer, images: &mut Images) {
    let blocks = match &reviewer.current {
        Some(current) => {
            let sheet = Stylesheet::parse(&current.css);
            let html = if current.revealed {
                &current.answer
            } else {
                &current.question
            };
            html_to_blocks(html, &sheet)
        }
        None => vec![Block::Text(vec![
            Line::from("Congratulations!").bold(),
            Line::from("Nothing more to review in this deck right now."),
        ])],
    };
    // Leave a margin so long text does not touch the edges.
    let inner = Rect {
        x: area.x + 2,
        y: area.y,
        width: area.width.saturating_sub(4).max(1),
        height: area.height,
    };
    blocks::draw(
        frame,
        inner,
        blocks,
        images,
        blocks::Options {
            align: Alignment::Center,
            vertical_center: true,
            scroll: 0,
        },
    );
}

fn draw_actions(frame: &mut Frame, area: Rect, reviewer: &Reviewer, status: Option<&str>) {
    let primary = match &reviewer.current {
        Some(current) if current.revealed => {
            let mut spans = vec![Span::raw(" ")];
            let answers = [
                ("1", "Again", AGAIN),
                ("2", "Hard", HARD),
                ("3", "Good", GOOD),
                ("4", "Easy", EASY),
            ];
            for (i, (key, name, color)) in answers.into_iter().enumerate() {
                spans.push(
                    Span::raw(format!(" {key} "))
                        .fg(Color::Black)
                        .bg(color)
                        .bold(),
                );
                spans.push(Span::raw(format!(" {name} ")).fg(color).bold());
                spans.push(Span::raw(current.labels[i].clone()).fg(color));
                spans.push(Span::raw("     "));
            }
            Line::from(spans)
        }
        Some(_) => Line::from(vec![
            Span::raw(" "),
            Span::raw(" space ").reversed().bold(),
            Span::raw(" show answer").bold(),
        ]),
        None => Line::from(vec![
            Span::raw(" "),
            Span::raw(" esc ").reversed().bold(),
            Span::raw(" back to decks").bold(),
        ]),
    };
    // The rest of the keys, esc and q among them, are one `?` away.
    let mut secondary =
        vec![Span::raw(" u undo   s suspend   b bury   f flag   m mark   e edit   ? help").dim()];
    if let Some(flag) = reviewer.current.as_ref().map(|c| c.flag).filter(|&f| f > 0) {
        secondary.push(Span::raw("   flag: ").dim());
        secondary.push(Span::raw(flag_name(flag)).fg(flag_color(flag)));
    }
    if reviewer.current.as_ref().is_some_and(|c| c.marked) {
        secondary.push(Span::raw("   ★ marked").fg(Color::Yellow));
    }
    if let Some(status) = status {
        secondary.push(Span::raw(format!("   {status}")).italic());
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![primary, Line::from(secondary)])),
        area,
    );
}

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
