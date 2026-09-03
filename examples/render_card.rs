//! Prints how the review screen would render one card, block by block with alignment
//! and styles, and tries to decode every image, so converter changes can be checked
//! against real notetypes:
//!
//! ```text
//! cargo run --example render_card -- ~/Library/Application\ Support/Anki2/User\ 1/collection.anki2 1524210625039 [--raw]
//! ```
//!
//! Card ids are listed by `yaac show NOTE_ID --json`. `--raw` also prints the HTML rslib
//! produced. Anki desktop must be closed.

use anki::card::CardId;
use anyhow::{Context, Result};
use yaac::config::Config;
use yaac::render::html::nodes_to_html;
use yaac::render::{Block, Stylesheet, html_to_blocks, image, occlusion};
use yaac::session::Session;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let raw = args.iter().any(|arg| arg == "--raw");
    let mut positional = args.iter().filter(|arg| !arg.starts_with("--"));
    let path = positional
        .next()
        .context("usage: render_card COLLECTION CARD_ID [--raw]")?;
    let card_id: i64 = positional
        .next()
        .context("usage: render_card COLLECTION CARD_ID [--raw]")?
        .parse()
        .context("card id must be a number")?;

    let mut session = Session::open(Some(path.as_ref()), &Config::default())?;
    let rendered = session
        .col
        .render_existing_card(CardId(card_id), false, false)
        .map_err(|err| anyhow::anyhow!("{err:?}"))?;
    let sheet = Stylesheet::parse(&rendered.css);
    let media_dir = session
        .path
        .parent()
        .map(|dir| dir.join("collection.media"))
        .unwrap_or_default();
    if raw {
        println!("== css ==\n{}", rendered.css);
    }
    for (side, nodes, answer_side) in [
        ("question", &rendered.qnodes, false),
        ("answer", &rendered.anodes, true),
    ] {
        let html = nodes_to_html(nodes, answer_side);
        if raw {
            println!("== {side} html ==\n{html}");
        }
        println!("== {side} ==");
        for block in html_to_blocks(&html, &sheet) {
            match block {
                Block::Text(lines) => {
                    for line in lines {
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
                Block::Image { src, align, masks } => {
                    let started = std::time::Instant::now();
                    let outcome = match image::load(&media_dir, &src) {
                        Ok(bitmap) => {
                            let mut note = format!("{}x{} px", bitmap.width(), bitmap.height());
                            if !masks.is_empty() {
                                // Occlusion cards: write the painted result next to the
                                // report so it can be looked at.
                                let painted = occlusion::apply(&bitmap, &masks);
                                let out = std::env::temp_dir().join(format!("yaac-{side}.png"));
                                painted.save(&out)?;
                                note.push_str(&format!(
                                    ", {} masks painted into {}",
                                    masks.len(),
                                    out.display()
                                ));
                            }
                            note
                        }
                        Err(err) => format!("failed: {err:#}"),
                    };
                    println!(
                        "{:<13} [image {src}: {outcome}, {} ms]",
                        format!("{align:?}"),
                        started.elapsed().as_millis()
                    );
                }
            }
        }
    }
    session.close()
}
