//! yaac as a library, so integration tests can drive the review engine and render the
//! screens without a terminal. The binary in `main.rs` only calls [`cli::run`].

pub mod cli;
pub mod config;
pub mod decks;
pub mod editor;
pub mod notes;
pub mod output;
pub mod render;
pub mod review;
pub mod session;
pub mod sync;
pub mod tui;
