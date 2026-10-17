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

    #[serde_inline_default(SizeMetric::Freed)]
    pub default_size_metric: SizeMetric,

    #[serde_inline_default(true)]
    pub default_show_size_cur_column: bool,

    #[serde_inline_default(0)]
    pub default_name_column_width: u64,

    #[serde_inline_default(0)]
    pub max_name_column_width: u64,

    #[serde_inline_default(true)]
    pub enable_delete_binding: bool,

    #[serde_inline_default(true)]
    pub require_delete_confirmation: bool,
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

/// The size metric used to display and sort entries. Sizes are disk usage
/// in bytes (allocated blocks, as in `du(1)`), not apparent file sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeMetric {
    /// The space that would be freed by recursively deleting the entry.
    /// A hard-linked file is counted exactly once, at the deepest directory
    /// that contains all of its names.
    Freed,
    /// Naive counting: every entry counts its full size at its own parent,
    /// so a hard-linked file is counted once per link.
    Naive,
}

impl SizeMetric {
    pub fn next(self) -> Self {
        match self {
            SizeMetric::Freed => SizeMetric::Naive,
            SizeMetric::Naive => SizeMetric::Freed,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SizeMetric::Freed => "freed",
            SizeMetric::Naive => "naive",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}
