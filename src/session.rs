use std::fmt;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use anki::collection::{Collection, CollectionBuilder};
use anki::error::{AnkiError, DbErrorKind};
use anki::prelude::I18n;
use anyhow::{Context, Result, bail};

use crate::config::Config;

const COLLECTION_FILE: &str = "collection.anki2";

/// An open collection plus where it came from. One per process.
pub struct Session {
    pub col: Collection,
    pub path: PathBuf,
    /// Run a sync on close if anything changed; set from the config's `auto_sync`.
    pub sync_on_close: bool,
}

#[derive(Debug)]
pub enum SessionError {
    Locked(PathBuf),
    NotFound(PathBuf),
    NoBaseDir,
    NoProfiles(PathBuf),
    MultipleProfiles(Vec<PathBuf>),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked(path) => write!(
                f,
                "{} is locked; Anki desktop is probably running. Quit it and retry",
                path.display()
            ),
            Self::NotFound(path) => write!(f, "collection not found at {}", path.display()),
            Self::NoBaseDir => write!(
                f,
                "cannot determine the Anki data directory; set ANKI_BASE or pass --collection"
            ),
            Self::NoProfiles(base) => {
                write!(
                    f,
                    "no profile with a {COLLECTION_FILE} under {}",
                    base.display()
                )
            }
            Self::MultipleProfiles(paths) => {
                writeln!(f, "several profiles found, pass --collection with one of:")?;
                for path in paths {
                    writeln!(f, "  {}", path.display())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SessionError {}

/// rslib errors carry their user-facing text behind a translation lookup.
pub fn anki_error(err: AnkiError) -> anyhow::Error {
    anyhow::anyhow!(err.message(&I18n::new(&["en"])))
}

/// Converts an rslib result into an anyhow one with a description of what was attempted.
pub trait AnkiResultExt<T> {
    fn ctx(self, what: &str) -> Result<T>;
}

impl<T> AnkiResultExt<T> for anki::error::Result<T> {
    fn ctx(self, what: &str) -> Result<T> {
        self.map_err(anki_error).context(what.to_string())
    }
}

impl Session {
    pub fn open(explicit: Option<&Path>, config: &Config) -> Result<Self> {
        let path = match explicit.or(config.collection.as_deref()) {
            Some(path) => path.to_path_buf(),
            None => discover_collection()?,
        };
        // rslib creates a collection at a missing path, which would silently turn a typo
        // into an empty collection. Only ever open files that already exist.
        if !path.is_file() {
            bail!(SessionError::NotFound(path));
        }
        let tr = I18n::new(&["en"]);
        let col = CollectionBuilder::new(&path)
            .with_desktop_media_paths()
            .set_tr(tr.clone())
            .build()
            .map_err(|err| match err {
                AnkiError::DbError { ref source } if source.kind == DbErrorKind::Locked => {
                    anyhow::Error::new(SessionError::Locked(path.clone()))
                }
                other => anyhow::anyhow!(other.message(&tr)),
            })
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self {
            col,
            path,
            sync_on_close: config.auto_sync,
        })
    }

    /// Takes a backup right now, regardless of the backup interval, and waits for it.
    pub fn backup_now(&mut self) -> Result<()> {
        if let Some(pending) = self.start_backup(true)? {
            finish_backup(pending)?;
        }
        Ok(())
    }

    /// Closes the collection, first taking a backup the way the desktop does on exit:
    /// only if something changed, and not more often than the collection's backup
    /// settings allow. With `auto_sync` on, changes are synced before closing; a failed
    /// sync is reported but does not fail the command, the changes are safe locally.
    pub fn close(mut self) -> Result<()> {
        let pending = self.start_backup(false)?;
        let changed = self.col.changes_since_open().ctx("counting changes")? > 0;
        if self.sync_on_close && changed {
            if let Err(err) = crate::sync::auto_sync(&mut self) {
                eprintln!("warning: auto sync failed: {err:#}");
            }
        }
        self.col
            .close(None)
            .ctx("closing collection")
            .with_context(|| format!("closing {}", self.path.display()))?;
        if let Some(handle) = pending {
            finish_backup(handle)?;
        }
        Ok(())
    }

    fn start_backup(&mut self, force: bool) -> Result<Option<BackupHandle>> {
        let Some(dir) = self.path.parent().map(|dir| dir.join("backups")) else {
            return Ok(None);
        };
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        self.col.maybe_backup(dir, force).ctx("starting backup")
    }
}

type BackupHandle = JoinHandle<anki::error::Result<()>>;

fn finish_backup(handle: BackupHandle) -> Result<()> {
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("backup thread panicked"))?
        .ctx("writing backup")
}

fn discover_collection() -> Result<PathBuf> {
    let base = base_dir().ok_or(SessionError::NoBaseDir)?;
    let mut profiles = profiles_in(&base)?;
    match profiles.len() {
        0 => bail!(SessionError::NoProfiles(base)),
        1 => Ok(profiles.remove(0)),
        _ => bail!(SessionError::MultipleProfiles(profiles)),
    }
}

/// Anki's own data directory, honouring the same `ANKI_BASE` override the desktop uses.
fn base_dir() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("ANKI_BASE") {
        return Some(PathBuf::from(base));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Anki2")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Anki2")
    } else {
        home.join(".local/share/Anki2")
    };
    Some(base)
}

/// Collection files of every profile folder under `base`, sorted for stable output.
fn profiles_in(base: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(base)
        .with_context(|| format!("reading Anki data directory {}", base.display()))?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join(COLLECTION_FILE))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    Ok(found)
}
