use std::fmt;
use std::path::{Path, PathBuf};

use anki::collection::{Collection, CollectionBuilder};
use anki::error::{AnkiError, DbErrorKind};
use anki::prelude::I18n;
use anyhow::{Context, Result, bail};

const COLLECTION_FILE: &str = "collection.anki2";

/// An open collection plus where it came from. One per process.
pub struct Session {
    pub col: Collection,
    pub path: PathBuf,
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

impl Session {
    pub fn open(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
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
        Ok(Self { col, path })
    }

    pub fn close(self) -> Result<()> {
        let tr = self.col.tr().clone();
        self.col
            .close(None)
            .map_err(|err| anyhow::anyhow!(err.message(&tr)))
            .with_context(|| format!("closing {}", self.path.display()))
    }
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
