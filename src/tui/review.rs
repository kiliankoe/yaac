//! The review screen: state on top, the card centered, answers along the bottom.

use std::time::Duration;

use anki::scheduler::answering::Rating;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::render::{Stylesheet, html_to_lines};
use crate::review::{Kind, Reviewer};
use crate::tui::{Terminal, next_key};

pub const AGAIN: Color = Color::Red;
pub const HARD: Color = Color::Yellow;
pub const GOOD: Color = Color::Green;
pub const EASY: Color = Color::Blue;

pub enum Action {
    Continue,
    Quit,
}

pub fn run(terminal: &mut Terminal, reviewer: &mut Reviewer) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, reviewer))?;
        if let Some(key) = next_key(Duration::from_millis(250))? {
            if let Action::Quit = handle(reviewer, key)? {
                return Ok(());
            }
        }
    }
}

pub fn handle(reviewer: &mut Reviewer, key: KeyEvent) -> Result<Action> {
    let revealed = reviewer.current.as_ref().is_some_and(|c| c.revealed);
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(Action::Quit);
        }
        KeyCode::Char(' ') | KeyCode::Enter => reviewer.reveal(),
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
        _ => {}
    }
    Ok(Action::Continue)
}

pub fn draw(frame: &mut Frame, reviewer: &Reviewer) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    draw_status(frame, top, reviewer);
    draw_card(frame, body, reviewer);
    draw_actions(frame, bottom, reviewer);
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

fn draw_card(frame: &mut Frame, area: Rect, reviewer: &Reviewer) {
    let lines = match &reviewer.current {
        Some(current) => {
            let sheet = Stylesheet::parse(&current.css);
            let html = if current.revealed {
                &current.answer
            } else {
                &current.question
            };
            html_to_lines(html, &sheet)
        }
        None => vec![
            Line::from("Congratulations!").bold(),
            Line::from("Nothing more to review in this deck right now."),
        ],
    };
    // Leave a margin so long text does not touch the edges.
    let inner = Rect {
        x: area.x + 2,
        y: area.y,
        width: area.width.saturating_sub(4).max(1),
        height: area.height,
    };
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center);
    let height = (paragraph.line_count(inner.width) as u16).min(inner.height);
    let centered = Rect {
        y: inner.y + (inner.height - height) / 2,
        height,
        ..inner
    };
    frame.render_widget(paragraph, centered);
}

fn draw_actions(frame: &mut Frame, area: Rect, reviewer: &Reviewer) {
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
        None => Line::from(" q quit").bold(),
    };
    let mut secondary = vec![Span::raw(" u undo   s suspend   b bury   f flag   q quit").dim()];
    if let Some(flag) = reviewer.current.as_ref().map(|c| c.flag).filter(|&f| f > 0) {
        secondary.push(Span::raw("   flag: ").dim());
        secondary.push(Span::raw(flag_name(flag)).fg(flag_color(flag)));
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![primary, Line::from(secondary)])),
        area,
    );
}

fn flag_name(flag: u32) -> &'static str {
    match flag {
        1 => "red",
        2 => "orange",
        3 => "green",
        4 => "blue",
        5 => "pink",
        6 => "turquoise",
        7 => "purple",
        _ => "none",
    }
}

fn flag_color(flag: u32) -> Color {
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
