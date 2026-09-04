//! Lays out the converter's text and image blocks in an area. The review card and the
//! browse detail pane share it.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_image::Image;

use crate::render::Block;
use crate::render::html::{image_label, stand_in};
use crate::render::occlusion::Mask;
use crate::tui::images::{Encoded, Images};

/// Lines longer than this are awkward to read, so on wide terminals the content gets
/// margins instead of the full width. The callers add their own padding at the screen
/// edges on top of this.
pub const MAX_WIDTH: u16 = 120;

/// Where the content sits in its area.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Alignment of text, and of images whose HTML gave none.
    pub align: Alignment,
    /// Center the content vertically when it is shorter than the area.
    pub vertical_center: bool,
    /// Rows scrolled off the top.
    pub scroll: u16,
}

/// A block ready to place: either wrapped text or an image of a known cell size.
enum Placed {
    Text(Box<Paragraph<'static>>, u16),
    Image {
        src: String,
        size: Size,
        align: Option<Alignment>,
        masks: Vec<Mask>,
    },
}

impl Placed {
    fn height(&self) -> u16 {
        match self {
            Self::Text(_, height) => *height,
            Self::Image { size, .. } => size.height,
        }
    }
}

/// Draws the blocks top to bottom and returns the height of the whole content, so the
/// caller can bound scrolling. Blocks above the scroll offset are skipped; a text block
/// cut by it shows its remaining rows, an image cut by it stays hidden.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    blocks: Vec<Block>,
    images: &mut Images,
    options: Options,
) -> u16 {
    let area = readable(area, options.align);
    let placed = place(blocks, area, images, options.align);
    let total: u16 = placed.iter().map(Placed::height).sum();
    let start = if options.vertical_center {
        area.y + area.height.saturating_sub(total) / 2
    } else {
        area.y
    };
    let bottom = i32::from(area.y + area.height);
    let mut y = i32::from(start) - i32::from(options.scroll);
    for block in placed {
        let top = y;
        y += i32::from(block.height());
        if y <= i32::from(area.y) {
            continue;
        }
        if top >= bottom {
            break;
        }
        let cut = (i32::from(area.y) - top).max(0) as u16;
        let row = (top + i32::from(cut)) as u16;
        let height = (block.height() - cut).min((bottom - i32::from(row)) as u16);
        match block {
            Placed::Text(paragraph, _) => {
                let rect = Rect {
                    x: area.x,
                    y: row,
                    width: area.width,
                    height,
                };
                frame.render_widget((*paragraph).scroll((cut, 0)), rect);
            }
            Placed::Image {
                src,
                size,
                align,
                masks,
            } => {
                if cut > 0 {
                    continue;
                }
                let x = match align.unwrap_or(options.align) {
                    Alignment::Left => area.x,
                    Alignment::Right => area.x + area.width - size.width,
                    Alignment::Center => area.x + (area.width - size.width) / 2,
                };
                let rect = Rect {
                    x,
                    y: row,
                    width: size.width,
                    height,
                };
                match images.protocol(&src, size, &masks) {
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
            }
        }
    }
    total
}

/// The area narrowed to `MAX_WIDTH`, kept to the side the content aligns to.
fn readable(area: Rect, align: Alignment) -> Rect {
    if area.width <= MAX_WIDTH {
        return area;
    }
    let x = match align {
        Alignment::Left => area.x,
        Alignment::Center => area.x + (area.width - MAX_WIDTH) / 2,
        Alignment::Right => area.x + area.width - MAX_WIDTH,
    };
    Rect {
        x,
        width: MAX_WIDTH,
        ..area
    }
}

/// Measures every block against the area. Images get at most half the height each and
/// never more than what the text leaves over; unusable images become labels.
fn place(blocks: Vec<Block>, area: Rect, images: &mut Images, align: Alignment) -> Vec<Placed> {
    let text_height = |lines: &[Line<'static>]| {
        Paragraph::new(Text::from(lines.to_vec()))
            .wrap(Wrap { trim: true })
            .line_count(area.width) as u16
    };
    let text_rows: u16 = blocks
        .iter()
        .map(|block| match block {
            Block::Text(lines) => text_height(lines),
            Block::Image { .. } | Block::Math { .. } => 0,
        })
        .sum();
    let image_count = blocks
        .iter()
        .filter(|block| matches!(block, Block::Image { .. } | Block::Math { .. }))
        .count()
        .max(1) as u16;
    let per_image = (area.height.saturating_sub(text_rows) / image_count)
        .min(area.height / 2)
        .max(1);

    let mut placed = Vec::new();
    let push_text = |placed: &mut Vec<Placed>, lines: Vec<Line<'static>>| {
        let height = text_height(&lines);
        let paragraph = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .alignment(align);
        placed.push(Placed::Text(Box::new(paragraph), height));
    };
    for block in blocks {
        match block {
            Block::Text(lines) => push_text(&mut placed, lines),
            Block::Image { src, align, masks } => {
                let available = Size::new(area.width, per_image);
                match images.size_for(&src, available) {
                    Some(size) => placed.push(Placed::Image {
                        src,
                        size,
                        align,
                        masks,
                    }),
                    None => {
                        let mut label = image_label(&src);
                        if let Some(problem) = images.problem(&src) {
                            label = format!("{label} ({problem})");
                        }
                        push_text(&mut placed, vec![stand_in(label, align)]);
                    }
                }
            }
            // A formula that cannot be drawn is readable as text, so no reason is added.
            Block::Math { math, align } => {
                let key = images.math(&math);
                let available = Size::new(area.width, per_image);
                match images.size_for(&key, available) {
                    Some(size) => placed.push(Placed::Image {
                        src: key,
                        size,
                        align,
                        masks: Vec::new(),
                    }),
                    None => push_text(&mut placed, vec![stand_in(math.text(), align)]),
                }
            }
        }
    }
    placed
}
