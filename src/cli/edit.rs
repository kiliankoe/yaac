use anyhow::{Result, bail};
use clap::Args;

use crate::cli::Context;
use crate::notes::{self, NoteDetails};
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Args)]
pub struct EditArgs {
    #[arg(value_name = "NOTE_ID")]
    id: String,

    /// New field values as NAME=VALUE, or bare values in field order. Fields not
    /// mentioned keep their content.
    #[arg(value_name = "FIELD", required = true)]
    fields: Vec<String>,
}

pub fn run(ctx: &Context, args: EditArgs) -> Result<()> {
    let nid = notes::note_ids(std::slice::from_ref(&args.id))?[0];
    let mut session = ctx.open()?;
    let col = &mut session.col;
    let mut note = notes::get_note(col, nid)?;
    let notetype = notes::get_notetype(col, &note)?;
    let changes = notes::parse_field_args(&notetype, &args.fields)?;
    if changes.is_empty() {
        bail!("no field values given");
    }
    for (idx, value) in changes {
        note.set_field(idx, value).ctx("setting field")?;
    }
    col.update_note(&mut note).ctx("updating note")?;
    let views = notes::views(col, &[nid])?;
    session.close()?;
    output::emit(&NoteDetails(views), ctx.json)
}
