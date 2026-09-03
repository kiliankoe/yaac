//! Turning Anki's rendered card HTML into terminal text.

pub mod css;
pub mod html;

pub use css::Stylesheet;
pub use html::html_to_lines;
