use std::path::PathBuf;

use serde::Deserialize;
use serde_default_utils::serde_inline_default;

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// Path to a theme toml file.
    /// Relative paths are resolved relative to the config directory, e.g. ~/.config/dumb/.
    /// If unspecified, the default theme is used.
    #[serde_inline_default(None)]
    pub theme: Option<PathBuf>,

    #[serde_inline_default(HeaderLayout::SingleLine)]
    pub default_header_layout: HeaderLayout,

    #[serde_inline_default(SortBy::Size)]
    pub default_sort_mode: SortBy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeaderLayout {
    SingleLine,
    MultiLine,
}

impl HeaderLayout {
    pub fn next(self) -> Self {
        match self {
            HeaderLayout::SingleLine => HeaderLayout::MultiLine,
            HeaderLayout::MultiLine => HeaderLayout::SingleLine,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
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
