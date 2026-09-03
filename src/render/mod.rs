//! Turning Anki's rendered card HTML into terminal text and images.

pub mod css;
pub mod html;
pub mod image;
pub mod occlusion;

pub use css::Stylesheet;
pub use html::{Block, html_to_blocks, html_to_lines};
