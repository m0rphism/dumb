use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Config {
    /// Path to a theme toml file.
    /// Relative paths are resolved relative to the config directory, e.g. ~/.config/dumb/.
    /// If unspecified, the default theme is used.
    pub theme: Option<PathBuf>,
}
