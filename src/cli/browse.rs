use anyhow::Result;
use clap::Args;

use crate::cli::Context;
use crate::tui;
use crate::tui::browse::Browser;
use crate::tui::images::{self, Images};

#[derive(Args)]
pub struct BrowseArgs {
    /// Anki search to start with (quoting is optional, words are joined). Without it
    /// the search box is focused.
    #[arg(value_name = "QUERY")]
    query: Vec<String>,
}

pub fn run(ctx: &Context, args: BrowseArgs) -> Result<()> {
    let mut session = ctx.open()?;
    // Probing writes to the terminal, so it happens before the alternate screen.
    let mut images = Images::new(
        images::probe(ctx.config.images.as_deref()),
        session.media_dir(),
    );
    let mut browser = Browser::new(args.query.join(" "));
    let mut terminal = tui::Terminal::open();
    tui::browse::run(&mut terminal, &mut session, &mut browser, &mut images)?;
    drop(terminal);
    session.close()
}
