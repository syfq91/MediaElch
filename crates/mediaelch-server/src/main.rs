//! Headless MediaElch server binary.

mod app;
mod cli;
mod config;

use std::error::Error;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use clap::Parser;
use cli::Cli;
use config::Config;
use mediaelch_db::Database;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let config = Config::load(&cli)?;

    if cli.check {
        println!("{}", config.summary());
        for warning in config.warnings() {
            println!("warning: {warning}");
        }
        return Ok(());
    }

    init_tracing(&config);

    std::fs::create_dir_all(&config.library.data_dir)?;
    let db_path = config.library.data_dir.join("mediaelch.sqlite");
    let db = Database::open(&db_path)?;

    let state = Arc::new(app::AppState {
        db: Arc::new(Mutex::new(db)),
        version: VERSION,
    });
    let listener = tokio::net::TcpListener::bind(config.bind_addr()?).await?;
    tracing::info!(address = %listener.local_addr()?, "HTTP server listening");

    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing(config: &Config) {
    let filter = tracing_subscriber::EnvFilter::builder().parse_lossy(&config.logging.level);
    if config.logging.json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

async fn shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("shutdown requested"),
        Err(error) => tracing::error!(%error, "failed to install Ctrl+C handler"),
    }
}
