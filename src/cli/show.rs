use anyhow::Result;
use clap::Args;

use crate::cli::Context;
use crate::notes::{self, NoteDetails};
use crate::output;

#[derive(Args)]
pub struct ShowArgs {
    /// Note ids, or "-" to read them from stdin.
    #[arg(value_name = "NOTE_ID", required = true)]
    ids: Vec<String>,
}

pub fn run(ctx: &Context, args: ShowArgs) -> Result<()> {
    let nids = notes::note_ids(&args.ids)?;
    let mut session = ctx.open()?;
    let views = notes::views(&mut session.col, &nids)?;
    session.close()?;
    output::emit(&NoteDetails(views), ctx.json)
}
