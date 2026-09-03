//! The review screen: state on top, the card centered, answers along the bottom.

use std::time::Duration;

use anki::scheduler::answering::Rating;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_image::Image;

use crate::render::html::image_label;
use crate::render::{Block, Stylesheet, html_to_blocks};
use crate::review::{Kind, Reviewer};
use crate::tui::images::{Encoded, Images};
use crate::tui::{Terminal, next_key};

pub const AGAIN: Color = Color::Red;
pub const HARD: Color = Color::Yellow;
pub const GOOD: Color = Color::Green;
pub const EASY: Color = Color::Blue;

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
    loop {
        terminal.draw(|frame| draw(frame, reviewer, images))?;
        images.end_frame();
        if let Some(key) = next_key(Duration::from_millis(250))? {
            if key.code == KeyCode::Char('r') {
                images.clear();
                continue;
            }
            match handle(reviewer, key)? {
                Action::Continue => {}
                action => return Ok(action),
            }
        }
    }
}

pub fn handle(reviewer: &mut Reviewer, key: KeyEvent) -> Result<Action> {
    let revealed = reviewer.current.as_ref().is_some_and(|c| c.revealed);
    match key.code {
        KeyCode::Char('q') => return Ok(Action::Quit),
        KeyCode::Esc => return Ok(Action::Back),
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

pub fn draw(frame: &mut Frame, reviewer: &Reviewer, images: &mut Images) {
    images.begin_frame();
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    draw_status(frame, top, reviewer);
    draw_card(frame, body, reviewer, images);
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

/// A block ready to place: either wrapped text or an image of a known cell size.
enum Placed {
    Text(Box<Paragraph<'static>>, u16),
    Image {
        src: String,
        size: Size,
        align: Option<Alignment>,
    },
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
    let placed = place(blocks, inner, images);
    let total: u16 = placed
        .iter()
        .map(|block| match block {
            Placed::Text(_, height) => *height,
            Placed::Image { size, .. } => size.height,
        })
        .sum();
    let mut y = inner.y + inner.height.saturating_sub(total) / 2;
    for block in placed {
        let remaining = (inner.y + inner.height).saturating_sub(y);
        if remaining == 0 {
            break;
        }
        match block {
            Placed::Text(paragraph, height) => {
                let rect = Rect {
                    y,
                    height: height.min(remaining),
                    ..inner
                };
                frame.render_widget(*paragraph, rect);
                y += height;
            }
            Placed::Image { src, size, align } => {
                let x = match align {
                    Some(Alignment::Left) => inner.x,
                    Some(Alignment::Right) => inner.x + inner.width - size.width,
                    _ => inner.x + (inner.width - size.width) / 2,
                };
                let rect = Rect {
                    x,
                    y,
                    width: size.width,
                    height: size.height.min(remaining),
                };
                match images.protocol(&src, size) {
                    Some(Encoded::Native(protocol)) => {
                        frame.render_widget(Image::new(protocol), rect);
                        images.mark_placed(&src, rect, None);
                    }
                    Some(Encoded::Kitty(placement)) => {
                        frame.render_widget(placement, rect);
                        let placement = placement.clone();
                        images.mark_placed(&src, rect, Some(placement));
                    }
                    None => {}
                }
                y += size.height;
            }
        }
    }
}

/// Measures every block against the area. Images get at most half the height each and
/// never more than what the text leaves over; unusable images become labels.
fn place(blocks: Vec<Block>, inner: Rect, images: &mut Images) -> Vec<Placed> {
    let text_height = |lines: &[Line<'static>]| {
        Paragraph::new(Text::from(lines.to_vec()))
            .wrap(Wrap { trim: true })
            .line_count(inner.width) as u16
    };
    let text_rows: u16 = blocks
        .iter()
        .map(|block| match block {
            Block::Text(lines) => text_height(lines),
            Block::Image { .. } => 0,
        })
        .sum();
    let image_count = blocks
        .iter()
        .filter(|block| matches!(block, Block::Image { .. }))
        .count()
        .max(1) as u16;
    let per_image = (inner.height.saturating_sub(text_rows) / image_count)
        .min(inner.height / 2)
        .max(1);

    let mut placed = Vec::new();
    let push_text = |placed: &mut Vec<Placed>, lines: Vec<Line<'static>>| {
        let height = text_height(&lines);
        let paragraph = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);
        placed.push(Placed::Text(Box::new(paragraph), height));
    };
    for block in blocks {
        match block {
            Block::Text(lines) => push_text(&mut placed, lines),
            Block::Image { src, align } => {
                let available = Size::new(inner.width, per_image);
                match images.size_for(&src, available) {
                    Some(size) => placed.push(Placed::Image { src, size, align }),
                    None => {
                        let mut label = image_label(&src);
                        if let Some(problem) = images.problem(&src) {
                            label = format!("{label} ({problem})");
                        }
                        let mut line =
                            Line::from(Span::styled(label, Style::new().fg(Color::Cyan)));
                        if let Some(align) = align {
                            line = line.alignment(align);
                        }
                        push_text(&mut placed, vec![line]);
                    }
                }
            }
        }
    }
    placed
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
        None => Line::from(vec![
            Span::raw(" "),
            Span::raw(" esc ").reversed().bold(),
            Span::raw(" back to decks").bold(),
        ]),
    };
    let mut secondary = vec![
        Span::raw(" u undo   s suspend   b bury   f flag   r redraw   esc decks   q quit").dim(),
    ];
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
