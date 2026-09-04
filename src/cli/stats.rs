use anyhow::Result;
use clap::Args;

use crate::cli::Context;
use crate::output;
use crate::stats;

#[derive(Args)]
pub struct StatsArgs {
    /// Anki search to limit the cards, e.g. 'deck:Spanish'; the whole collection
    /// without it.
    #[arg(value_name = "QUERY")]
    query: Vec<String>,

    /// Cover all history instead of the last 12 months.
    #[arg(long)]
    all: bool,
}

pub fn run(ctx: &Context, args: StatsArgs) -> Result<()> {
    let mut session = ctx.open()?;
    let stats = stats::collect(&mut session.col, &args.query.join(" "), args.all)?;
    session.close()?;
    output::emit(&stats, ctx.json)
}
