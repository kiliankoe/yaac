use std::fmt::Display;
use std::io::Write;

use anyhow::Result;
use serde::Serialize;

/// Every command renders through here so `--json` behaves the same everywhere.
pub fn emit<T: Serialize + Display>(value: &T, json: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut stdout, value)?;
        writeln!(stdout)?;
    } else {
        write!(stdout, "{value}")?;
    }
    Ok(())
}
