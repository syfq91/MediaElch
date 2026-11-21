//! Layered configuration: built-in defaults < TOML file < environment < CLI.

use std::net::{AddrParseError, SocketAddr};
use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cli::Cli;

const ENV_PREFIX: &str = "MEDIAELCH_";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration: {0}")]
    Figment(Box<figment::Error>),
    #[error("invalid HTTP bind address {value:?}: {source}")]
    InvalidBind {
        value: String,
        source: AddrParseError,
    },
    #[error("invalid log level {value:?}; expected trace, debug, info, warn or error")]
    InvalidLogLevel { value: String },
    #[error("data_dir must not be empty")]
    EmptyDataDir,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub library: LibraryConfig,
    pub scrapers: ScraperConfig,
    pub images: ImageConfig,
    pub logging: LoggingConfig,
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryConfig {
    pub data_dir: PathBuf,
    pub media_directories: Vec<PathBuf>,
    pub scan_interval_minutes: u64,
    pub write_nfo_automatically: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScraperConfig {
    pub tmdb_api_key: Option<String>,
    pub fanarttv_api_key: Option<String>,
    pub fanarttv_client_key: Option<String>,
    pub language: String,
    pub proxy: Option<String>,
    pub proxy_bypass_local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageConfig {
    pub max_backdrop_width: u32,
    pub max_poster_width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub json: bool,
}

impl From<figment::Error> for ConfigError {
    fn from(error: figment::Error) -> Self {
        Self::Figment(Box::new(error))
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".to_owned(),
            base_url: None,
        }
    }
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            media_directories: Vec::new(),
            scan_interval_minutes: 0,
            write_nfo_automatically: false,
        }
    }
}

impl Default for ScraperConfig {
    fn default() -> Self {
        Self {
            tmdb_api_key: None,
            fanarttv_api_key: None,
            fanarttv_client_key: None,
            language: "en-US".to_owned(),
            proxy: None,
            proxy_bypass_local: true,
        }
    }
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_backdrop_width: 1920,
            max_poster_width: 1000,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            json: false,
        }
    }
}

impl Config {
    /// Load defaults, then merge TOML, environment and CLI overrides (in that
    /// order) and validate the result.
    pub fn load(cli: &Cli) -> Result<Self, ConfigError> {
        let path = cli.config.clone().unwrap_or_else(default_config_path);

        let mut figment = Figment::from(Toml::string(""));
        if path.exists() {
            figment = figment.merge(Toml::file(&path));
        }
        figment = figment.merge(Env::prefixed(ENV_PREFIX).split("__"));

        let mut config: Self = figment.extract()?;
        config.config_path = Some(path);

        if let Some(bind) = &cli.bind {
            config.server.bind = bind.clone();
        }
        if let Some(data_dir) = &cli.data_dir {
            config.library.data_dir = data_dir.clone();
        }
        if let Some(level) = &cli.log_level {
            config.logging.level = level.clone();
        }

        config.validate()?;
        Ok(config)
    }

    /// Validate values that would otherwise fail much later at runtime.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.bind_addr()?;

        const LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];
        if !LEVELS.contains(&self.logging.level.as_str()) {
            return Err(ConfigError::InvalidLogLevel {
                value: self.logging.level.clone(),
            });
        }

        if self.library.data_dir.as_os_str().is_empty() {
            return Err(ConfigError::EmptyDataDir);
        }
        Ok(())
    }

    pub fn bind_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.server
            .bind
            .parse()
            .map_err(|source| ConfigError::InvalidBind {
                value: self.server.bind.clone(),
                source,
            })
    }

    /// Non-fatal configuration issues worth showing in `--check` output.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.library.media_directories.is_empty() {
            warnings.push("no media_directories configured; the library will be empty".to_owned());
        }
        if self.scrapers.tmdb_api_key.is_none() {
            warnings.push(
                "scrapers.tmdb_api_key is not set; TMDB scraping will be unavailable".to_owned(),
            );
        }
        if self.scrapers.fanarttv_api_key.is_none() {
            warnings.push(
                "scrapers.fanarttv_api_key is not set; fanart.tv artwork will be unavailable"
                    .to_owned(),
            );
        }
        warnings
    }

    /// Human-readable summary with secrets redacted.
    pub fn summary(&self) -> String {
        let path = self
            .config_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<defaults>".to_owned());
        format!(
            "config file: {path}\nbind: {}\ndata dir: {}\nmedia directories: {}\nlog level: {} (json: {})\nTMDB key: {}\nfanart.tv key: {}",
            self.server.bind,
            self.library.data_dir.display(),
            self.library.media_directories.len(),
            self.logging.level,
            self.logging.json,
            redact(&self.scrapers.tmdb_api_key),
            redact(&self.scrapers.fanarttv_api_key),
        )
    }

    /// Build a configuration from an arbitrary Figment provider chain.
    /// Used by tests to avoid mutating process environment variables.
    #[cfg(test)]
    fn from_figment(figment: Figment) -> Result<Self, ConfigError> {
        let config: Self = figment.extract()?;
        config.validate()?;
        Ok(config)
    }
}

fn redact(value: &Option<String>) -> &'static str {
    match value {
        Some(v) if v.is_empty() => "<empty>",
        Some(_) => "<set>",
        None => "<unset>",
    }
}

fn default_config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("mediaelch").join("config.toml");
    }
    if let Ok(home) = std::env::var("HOME") {
        return Path::new(&home)
            .join(".config")
            .join("mediaelch")
            .join("config.toml");
    }
    PathBuf::from("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bind_to_loopback_and_are_valid() {
        let config = Config::default();
        config.validate().unwrap();
        assert_eq!(config.bind_addr().unwrap().to_string(), "127.0.0.1:8080");
        assert!(config.library.media_directories.is_empty());
        assert!(!config.library.write_nfo_automatically);
    }

    #[test]
    fn toml_values_are_read_and_nested_defaults_preserved() {
        let config = Config::from_figment(Figment::from(Toml::string(
            r#"
            [server]
            bind = "0.0.0.0:9000"

            [library]
            media_directories = ["/media/movies"]

            [scrapers]
            tmdb_api_key = "secret"
            "#,
        )))
        .unwrap();

        assert_eq!(config.server.bind, "0.0.0.0:9000");
        assert_eq!(
            config.library.media_directories,
            vec![PathBuf::from("/media/movies")]
        );
        assert_eq!(config.scrapers.tmdb_api_key.as_deref(), Some("secret"));
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.images.max_poster_width, 1000);
    }

    #[test]
    fn invalid_bind_address_is_rejected() {
        let config = Config::from_figment(Figment::from(Toml::string(
            "[server]\nbind = \"not-an-address\"\n",
        )));
        assert!(matches!(config, Err(ConfigError::InvalidBind { .. })));
    }

    #[test]
    fn invalid_log_level_is_rejected() {
        let config = Config::from_figment(Figment::from(Toml::string(
            "[logging]\nlevel = \"verbose\"\n",
        )));
        assert!(matches!(config, Err(ConfigError::InvalidLogLevel { .. })));
    }

    #[test]
    fn summary_redacts_api_keys() {
        let mut config = Config::default();
        config.scrapers.tmdb_api_key = Some("super-secret".to_owned());
        let summary = config.summary();
        assert!(summary.contains("TMDB key: <set>"));
        assert!(!summary.contains("super-secret"));
    }

    #[test]
    fn warnings_report_missing_setup() {
        let warnings = Config::default().warnings();
        assert_eq!(warnings.len(), 3);
    }
}
