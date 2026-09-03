use std::fmt;

use anki::notetype::NotetypeKind;
use anyhow::{Context as _, Result};
use serde::Serialize;

use crate::cli::Context;
use crate::output;
use crate::session::AnkiResultExt;

#[derive(Serialize)]
#[serde(transparent)]
struct NotetypeList(Vec<NotetypeRow>);

#[derive(Serialize)]
struct NotetypeRow {
    id: i64,
    name: String,
    kind: &'static str,
    fields: Vec<String>,
    templates: Vec<String>,
}

pub fn run(ctx: &Context) -> Result<()> {
    let mut session = ctx.open()?;
    let col = &mut session.col;
    let mut rows = Vec::new();
    for (id, name) in col
        .storage
        .get_all_notetype_names()
        .ctx("listing notetypes")?
    {
        let notetype = col
            .get_notetype(id)
            .ctx("reading notetype")?
            .with_context(|| format!("notetype {name} vanished while listing"))?;
        rows.push(NotetypeRow {
            id: id.0,
            name,
            kind: match notetype.config.kind() {
                NotetypeKind::Cloze => "cloze",
                NotetypeKind::Normal => "normal",
            },
            fields: notetype
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            templates: notetype
                .templates
                .iter()
                .map(|template| template.name.clone())
                .collect(),
        });
    }
    session.close()?;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    output::emit(&NotetypeList(rows), ctx.json)
}

impl fmt::Display for NotetypeList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for notetype in &self.0 {
            writeln!(
                f,
                "{}  ({})\n  fields: {}\n  cards:  {}",
                notetype.name,
                notetype.kind,
                notetype.fields.join(", "),
                notetype.templates.join(", ")
            )?;
        }
        Ok(())
    }
}
