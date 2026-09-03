use anki::browser_table::Column;
use anki::search::SortMode;
use anyhow::Result;
use clap::Args;

use crate::cli::Context;
use crate::notes::{self, NoteList};
use crate::output;
use crate::session::AnkiResultExt;

/// Shown under `search --help`; kept close to the manual so it can double as a cheat sheet.
const SYNTAX_HELP: &str = "\
Search syntax (terms are ANDed; OR, -NOT and parentheses group and negate):
  cat                       any field contains cat; \"two words\" keeps them together
  front:cat                 field Front is exactly cat; front:cat* prefix, front:*cat* contains
  front:_at                 _ matches one character; front:* any non-empty, front: empty
  deck:Spanish              this deck and its subdecks; deck:Spanish::Verbs for one subdeck
  note:Basic  card:1        notetype; card by template number or card:\"Card 2\" by name
  tag:vocab  tag:none       has tag (including child tags like vocab::animals); untagged
  is:due  is:new  is:learn  is:review  is:suspended  is:buried
  flag:1                    1 red, 2 orange, 3 green, 4 blue, 5 pink, 6 turquoise, 7 purple
  prop:due=0                due today; prop:due<=3 within 3 days; prop:due=-1 overdue since yesterday
  prop:ivl>=30  prop:reps>10  prop:lapses>3  prop:ease<2   interval days, reviews, lapses, ease
  added:7  edited:7         added / edited within the last 7 days
  rated:7  rated:30:1       reviewed in the last 7 days / answered Again in the last 30 days
  introduced:30             first reviewed in the last 30 days
  nid:1234  cid:5678        by note id / card id
  re:^\\d+$                  regular expression over all fields; front:re:... for one field
  w:cat  nc:uber            whole word only; ignore accents (matches \u{fc}ber)
  -tag:done  -is:suspended  exclude
  (deck:Spanish OR deck:French) is:due

Full reference: https://docs.ankiweb.net/searching.html";

#[derive(Args)]
#[command(after_help = SYNTAX_HELP)]
pub struct SearchArgs {
    /// Anki search, e.g. 'deck:Spanish is:due' (quoting is optional, words are joined).
    #[arg(value_name = "QUERY", required = true)]
    query: Vec<String>,

    /// Stop after this many notes.
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Print only note ids, one per line, for piping into other commands.
    #[arg(long, conflicts_with = "json")]
    ids: bool,
}

pub fn run(ctx: &Context, args: SearchArgs) -> Result<()> {
    let mut session = ctx.open()?;
    let query = args.query.join(" ");
    let by_creation = SortMode::Builtin {
        column: Column::NoteCreation,
        reverse: false,
    };
    let mut nids = session
        .col
        .search_notes(query.as_str(), by_creation)
        .ctx("searching")?;
    if let Some(limit) = args.limit {
        nids.truncate(limit);
    }
    if args.ids {
        session.close()?;
        for nid in nids {
            println!("{}", nid.0);
        }
        return Ok(());
    }
    let views = notes::views(&mut session.col, &nids)?;
    session.close()?;
    output::emit(&NoteList(views), ctx.json)
}
