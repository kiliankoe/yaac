//! Prints how the review screen would render one card, line by line with alignment
//! and styles, so converter changes can be checked against real notetypes:
//!
//! ```text
//! cargo run --example render_card -- ~/Library/Application\ Support/Anki2/User\ 1/collection.anki2 1524210625039
//! ```
//!
//! Card ids are listed by `yaac show NOTE_ID --json`. Anki desktop must be closed.

use anki::card::CardId;
use anyhow::{Context, Result};
use yaac::config::Config;
use yaac::render::html::nodes_to_html;
use yaac::render::{Stylesheet, html_to_lines};
use yaac::session::Session;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .context("usage: render_card COLLECTION CARD_ID")?;
    let card_id: i64 = args
        .next()
        .context("usage: render_card COLLECTION CARD_ID")?
        .parse()
        .context("card id must be a number")?;

    let mut session = Session::open(Some(path.as_ref()), &Config::default())?;
    let rendered = session
        .col
        .render_existing_card(CardId(card_id), false, false)
        .map_err(|err| anyhow::anyhow!("{err:?}"))?;
    let sheet = Stylesheet::parse(&rendered.css);
    for (side, nodes, answer_side) in [
        ("question", &rendered.qnodes, false),
        ("answer", &rendered.anodes, true),
    ] {
        println!("== {side} ==");
        for line in html_to_lines(&nodes_to_html(nodes, answer_side), &sheet) {
            let spans: Vec<String> = line
                .spans
                .iter()
                .map(|span| format!("{:?} {:?}", span.content, span.style))
                .collect();
            println!(
                "{:<13} {}",
                format!("{:?}", line.alignment),
                spans.join("  +  ")
            );
        }
    }
    session.close()
}
