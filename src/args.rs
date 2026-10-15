use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the config file.
    ///
    /// Defaults to `XDG_CONFIG_DIR/dumb/config.toml` or if not set `~/.config/dumb/config.toml`.
    ///
    /// Relative paths specified inside the config file are resolved with respect to the parent
    /// directory of this path.
    #[arg(short, long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Directory or file to calculate disk usage for.
    #[arg(value_name = "PATH")]
    pub target: Option<PathBuf>,
}
