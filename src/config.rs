use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::render::latex;

/// Settings from `$YAAC_CONFIG`, else `$XDG_CONFIG_HOME/yaac/config.toml`, else
/// `~/.config/yaac/config.toml`. A missing file means defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub collection: Option<PathBuf>,
    pub default_notetype: Option<String>,
    pub default_deck: Option<String>,
    /// Sync after every command that changed something.
    #[serde(default)]
    pub auto_sync: bool,
    /// Self-hosted sync server URL; absent means AnkiWeb.
    pub sync_endpoint: Option<String>,
    /// Terminal graphics: auto, kitty, sixel, iterm2, halfblocks, or off.
    pub images: Option<String>,
    /// Ink for typeset formulas, `#rrggbb` or a CSS name; absent means chosen from the
    /// terminal's background.
    pub latex_colour: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let explicit = std::env::var_os("YAAC_CONFIG").map(PathBuf::from);
        let Some(path) = explicit.clone().or_else(default_path) else {
            return Ok(Self::default());
        };
        if !path.is_file() {
            if explicit.is_some() {
                bail!("config file {} does not exist", path.display());
            }
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

impl Config {
    pub fn latex_colour(&self) -> Result<Option<[u8; 3]>> {
        self.latex_colour
            .as_deref()
            .map(|text| {
                latex::parse_colour(text).with_context(|| {
                    format!("latex_colour {text:?} is not #rrggbb or a colour name")
                })
            })
            .transpose()
    }
}

fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("yaac").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latex_colour_is_optional_and_checked() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.latex_colour().unwrap(), None);
        let config: Config = toml::from_str("latex_colour = \"#abcdef\"").unwrap();
        assert_eq!(config.latex_colour().unwrap(), Some([0xab, 0xcd, 0xef]));
        let config: Config = toml::from_str("latex_colour = \"blurple\"").unwrap();
        assert!(config.latex_colour().is_err());
    }
}
