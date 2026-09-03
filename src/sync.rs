//! AnkiWeb (or self-hosted sync server) access: stored credentials and the collection,
//! full, and media sync steps. Everything async from rslib is confined to this module.

use std::path::PathBuf;

use anki::sync::collection::normal::SyncActionRequired;
use anki::sync::login::{SyncAuth, sync_login};
use anki::sync::media::progress::MediaSyncProgress;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use crate::session::{AnkiResultExt, Session, anki_error};

/// What `login` stores: the host key AnkiWeb hands out, never the password. Kept apart
/// from the config file so the config can live in dotfiles.
#[derive(Debug, Serialize, Deserialize)]
pub struct Auth {
    pub username: String,
    pub hkey: String,
    /// Sync server base URL; absent means AnkiWeb. Updated when the server redirects.
    pub endpoint: Option<String>,
}

pub fn auth_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("YAAC_AUTH") {
        return Ok(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .context("cannot determine a data directory; set XDG_DATA_HOME or YAAC_AUTH")?;
    Ok(base.join("yaac").join("auth.toml"))
}

pub fn load_auth() -> Result<Option<Auth>> {
    let path = auth_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text)
        .map(Some)
        .with_context(|| format!("parsing {}", path.display()))
}

pub fn require_auth() -> Result<Auth> {
    load_auth()?.context("not logged in; run `yaac login` first")
}

pub fn save_auth(auth: &Auth) -> Result<PathBuf> {
    let path = auth_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = toml::to_string(auth).context("serialising credentials")?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restricting permissions on {}", path.display()))?;
    }
    Ok(path)
}

/// Removes stored credentials; true if there were any.
pub fn forget_auth() -> Result<bool> {
    let path = auth_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

fn runtime() -> Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")
}

fn sync_auth(auth: &Auth) -> Result<SyncAuth> {
    let endpoint = auth
        .endpoint
        .as_deref()
        .map(Url::parse)
        .transpose()
        .context("invalid sync endpoint")?;
    Ok(SyncAuth {
        hkey: auth.hkey.clone(),
        endpoint,
        io_timeout_secs: None,
    })
}

pub fn login(username: &str, password: &str, endpoint: Option<&str>) -> Result<Auth> {
    let rt = runtime()?;
    let auth = rt
        .block_on(sync_login(
            username.to_string(),
            password.to_string(),
            endpoint.map(str::to_string),
            Client::new(),
        ))
        .map_err(anki_error)
        .context("logging in")?;
    // rslib returns only the key; the endpoint we logged in against is the one to keep.
    Ok(Auth {
        username: username.to_string(),
        hkey: auth.hkey,
        endpoint: endpoint.map(str::to_string),
    })
}

pub enum NormalOutcome {
    /// Both sides agree now; `changed` says whether anything moved in either direction.
    Done { changed: bool },
    /// The collections diverged too far for a normal sync; the user must pick a side.
    FullSyncRequired { upload_ok: bool, download_ok: bool },
}

pub struct NormalReport {
    pub outcome: NormalOutcome,
    pub server_message: String,
}

/// A normal sync. Records a redirected endpoint in `auth`, which the caller should save.
pub fn normal(session: &mut Session, auth: &mut Auth) -> Result<NormalReport> {
    let rt = runtime()?;
    let before = session.col.changes_since_open().ctx("counting changes")?;
    let output = rt
        .block_on(session.col.normal_sync(sync_auth(auth)?, Client::new()))
        .map_err(anki_error)
        .context("syncing collection")?;
    if let Some(endpoint) = output.new_endpoint {
        auth.endpoint = Some(endpoint);
    }
    let outcome = match output.required {
        SyncActionRequired::FullSyncRequired {
            upload_ok,
            download_ok,
        } => NormalOutcome::FullSyncRequired {
            upload_ok,
            download_ok,
        },
        _ => {
            let after = session.col.changes_since_open().ctx("counting changes")?;
            NormalOutcome::Done {
                changed: after > before,
            }
        }
    };
    Ok(NormalReport {
        outcome,
        server_message: output.server_message,
    })
}

/// Replaces one side with the other. Takes the session because rslib closes the
/// collection to copy the file, and hands back a reopened one.
pub fn full(session: Session, auth: &Auth, upload: bool) -> Result<Session> {
    let rt = runtime()?;
    let Session {
        col,
        path,
        sync_on_close,
    } = session;
    let mut builder = col.as_builder();
    let result = if upload {
        rt.block_on(col.full_upload(sync_auth(auth)?, Client::new()))
    } else {
        rt.block_on(col.full_download(sync_auth(auth)?, Client::new()))
    };
    let col = builder
        .build()
        .ctx("reopening collection after full sync")?;
    let session = Session {
        col,
        path,
        sync_on_close,
    };
    result.map_err(anki_error).context(if upload {
        "uploading collection"
    } else {
        "downloading collection"
    })?;
    Ok(session)
}

/// Syncs the media folder. rslib's progress counters live in a private type, so only
/// success or failure is reported.
pub fn media(session: &mut Session, auth: &Auth) -> Result<()> {
    let rt = runtime()?;
    let manager = session.col.media().ctx("opening media folder")?;
    let handler = session.col.new_progress_handler::<MediaSyncProgress>();
    rt.block_on(manager.sync_media(handler, sync_auth(auth)?, Client::new(), None))
        .map_err(anki_error)
        .context("syncing media")
}

/// Runs after mutating commands when `auto_sync` is on. Failures are reported, not
/// fatal: the change is safely in the local collection and the next sync picks it up.
pub fn auto_sync(session: &mut Session) -> Result<()> {
    let Some(mut auth) = load_auth()? else {
        eprintln!("auto_sync is on but you are not logged in; run `yaac login`");
        return Ok(());
    };
    let report = normal(session, &mut auth)?;
    match report.outcome {
        NormalOutcome::Done { .. } => {}
        NormalOutcome::FullSyncRequired { .. } => {
            bail!("a full sync is required; run `yaac sync` to choose a direction")
        }
    }
    media(session, &auth)?;
    save_auth(&auth)?;
    Ok(())
}
