use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Directory or file to calculate disk usage for.
    #[arg(value_name = "PATH")]
    pub target: Option<PathBuf>,
}

// #[arg(short, long, value_name = "PATH")]
// #[arg(trailing_var_arg = true)]
