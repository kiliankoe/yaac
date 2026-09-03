use std::fmt;

use anyhow::Result;
use serde::Serialize;

use crate::cli::Context;
use crate::output;
use crate::sync;

#[derive(Serialize)]
struct LoggedOut {
    removed: bool,
}

pub fn run(ctx: &Context) -> Result<()> {
    let removed = sync::forget_auth()?;
    output::emit(&LoggedOut { removed }, ctx.json)
}

impl fmt::Display for LoggedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.removed {
            writeln!(f, "stored credentials removed")
        } else {
            writeln!(f, "no stored credentials")
        }
    }
}
