//! Boxes drawn over a screen: the key list every screen shows on `?`, and the frame
//! around the deck picker's notetype chooser.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph};

/// A cleared, bordered box centered on the screen, sized for `width` by `height` cells
/// of content plus a space on either side, and wide enough for its title and the hint
/// along the bottom. Returns the area inside the border; a small screen cuts the box
/// rather than the screen.
pub fn boxed(frame: &mut Frame, title: &str, hint: &str, width: u16, height: u16) -> Rect {
    let screen = frame.area();
    let title = format!(" {title} ");
    let hint = format!(" {hint} ");
    let width = (width + 2)
        .max(title.chars().count() as u16)
        .max(hint.chars().count() as u16)
        .saturating_add(2)
        .min(screen.width);
    let height = height.saturating_add(2).min(screen.height);
    let area = Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Line::from(title).bold())
        .title_bottom(Line::from(hint).dim().right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// The keys of a screen, one per row, closed by the next key press.
pub fn keys(frame: &mut Frame, title: &str, bindings: &[(&str, &str)]) {
    let width_of = |text: &str| text.chars().count();
    let key_width = bindings
        .iter()
        .map(|(key, _)| width_of(key))
        .max()
        .unwrap_or(0);
    let action_width = bindings
        .iter()
        .map(|(_, action)| width_of(action))
        .max()
        .unwrap_or(0);
    let inner = boxed(
        frame,
        title,
        "any key closes",
        (key_width + 2 + action_width) as u16,
        bindings.len() as u16,
    );
    let lines: Vec<Line> = bindings
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::raw(format!(" {key:<key_width$}  ")).bold(),
                Span::raw(*action),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}
