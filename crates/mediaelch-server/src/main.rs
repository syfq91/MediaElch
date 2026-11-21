//! Headless MediaElch server binary.

mod api;
mod app;
mod auth;
mod cli;
mod config;
mod scrape_api;
mod ui;
mod write_api;

use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use cli::Cli;
use config::Config;
use mediaelch_db::Database;
use mediaelch_fs::ScanRoot;

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
    if let Some(password) = &cli.hash_password {
        println!("{}", auth::AuthState::hash_password(password)?);
        return Ok(());
    }
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

    let state = Arc::new(app::AppState::new(
        db,
        VERSION,
        roots_from_config(&config),
        config.library.data_dir.clone(),
        mediaelch_image::ImageConfig {
            max_poster_width: config.images.max_poster_width,
            max_backdrop_width: config.images.max_backdrop_width,
        },
        Arc::new(scrape_api::ScrapeRegistry::new(
            mediaelch_scrape::ScrapeConfig {
                tmdb_api_key: config.scrapers.tmdb_api_key.clone(),
                fanarttv_api_key: config.scrapers.fanarttv_api_key.clone(),
                fanarttv_client_key: config.scrapers.fanarttv_client_key.clone(),
                language: config.scrapers.language.clone(),
                proxy: config.scrapers.proxy.clone(),
                ..mediaelch_scrape::ScrapeConfig::default()
            },
        )),
        Arc::new(auth::AuthState::new(
            config.auth.enabled,
            config.auth.password_hash.clone(),
        )),
    ));
    let router = app::router(state);
    let address = config.bind_addr()?;

    if let (Some(cert), Some(key)) = (&config.server.tls_cert, &config.server.tls_key) {
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
        let handle = axum_server::Handle::new();
        let shutdown = handle.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
        });
        tracing::info!(%address, "HTTPS server listening");
        axum_server::bind_rustls(address, tls)
            .handle(handle)
            .serve(router.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(address).await?;
        tracing::info!(address = %listener.local_addr()?, "HTTP server listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
    }

    Ok(())
}

fn roots_from_config(config: &Config) -> Vec<ScanRoot> {
    config
        .library
        .media_directories
        .iter()
        .map(|path: &PathBuf| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let kind = if name.contains("concert") {
                "concert"
            } else if name.contains("show") || name.contains("tv") || name.contains("serie") {
                "tvshow"
            } else if name.contains("movie") || name.contains("film") {
                "movie"
            } else {
                "auto"
            };
            ScanRoot {
                path: path.clone(),
                kind: kind.to_owned(),
                recursive: true,
            }
        })
        .collect()
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
