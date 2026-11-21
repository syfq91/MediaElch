//! Command line interface for the headless server.

use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "mediaelch-server",
    version,
    about = "Headless MediaElch server for Kodi media libraries"
)]
pub struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, env = "MEDIAELCH_CONFIG")]
    pub config: Option<PathBuf>,

    /// Override the HTTP bind address, e.g. 0.0.0.0:8080.
    #[arg(long)]
    pub bind: Option<String>,

    /// Override the directory used for the SQLite database and generated files.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Override the log level (trace, debug, info, warn, error).
    #[arg(long)]
    pub log_level: Option<String>,

    /// Validate the configuration and exit without starting the server.
    #[arg(long)]
    pub check: bool,

    /// Print an Argon2id hash for the given password and exit.
    #[arg(long, value_name = "PASSWORD")]
    pub hash_password: Option<String>,
}
