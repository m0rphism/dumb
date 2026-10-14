use std::path::PathBuf;

use serde::Deserialize;
use serde_default_utils::serde_inline_default;

#[serde_inline_default]
#[derive(Deserialize)]
pub struct Config {
    /// Path to a theme toml file.
    /// Relative paths are resolved relative to the config directory, e.g. ~/.config/dumb/.
    /// If unspecified, the default theme is used.
    #[serde_inline_default(None)]
    pub theme: Option<PathBuf>,

    #[serde_inline_default(SortBy::Size)]
    pub default_sort_mode: SortBy,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum SortBy {
    Name,
    Size,
    ModTime,
}
impl SortBy {
    pub fn next(self) -> Self {
        match self {
            SortBy::Size => SortBy::Name,
            SortBy::Name => SortBy::ModTime,
            SortBy::ModTime => SortBy::Size,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}
