use std::fmt;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde::Serialize;

use crate::cli::Context;
use crate::output;
use crate::sync;

#[derive(Args)]
pub struct LoginArgs {
    /// AnkiWeb username (email). Prompted for when omitted.
    #[arg(value_name = "USERNAME")]
    username: Option<String>,

    /// Sync server URL for self-hosted servers; falls back to sync_endpoint in the
    /// config, then AnkiWeb.
    #[arg(long, value_name = "URL")]
    endpoint: Option<String>,
}

#[derive(Serialize)]
struct LoggedIn {
    username: String,
    endpoint: Option<String>,
    auth_file: PathBuf,
}

pub fn run(ctx: &Context, args: LoginArgs) -> Result<()> {
    let username = match args.username {
        Some(username) => username,
        None => prompt_line("AnkiWeb username: ")?,
    };
    let password = read_password()?;
    if username.is_empty() || password.is_empty() {
        bail!("username and password are required");
    }
    let endpoint = args
        .endpoint
        .as_deref()
        .or(ctx.config.sync_endpoint.as_deref());
    let auth = sync::login(&username, &password, endpoint)?;
    let auth_file = sync::save_auth(&auth)?;
    output::emit(
        &LoggedIn {
            username,
            endpoint: auth.endpoint,
            auth_file,
        },
        ctx.json,
    )
}

fn prompt_line(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading input")?;
    Ok(line.trim().to_string())
}

/// Hidden prompt on a terminal; a plain line from stdin when piped, so scripts can
/// feed the password without putting it on the command line.
fn read_password() -> Result<String> {
    if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Password: ").context("reading password")
    } else {
        prompt_line("")
    }
}

impl fmt::Display for LoggedIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "logged in as {} ({}); credentials stored in {}",
            self.username,
            self.endpoint.as_deref().unwrap_or("AnkiWeb"),
            self.auth_file.display()
        )
    }
}
