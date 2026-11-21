//! Scraper traits and shared HTTP/cache infrastructure.

pub mod fanarttv;
pub mod tmdb;
pub mod tvmaze;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use mediaelch_core::{ExternalIds, ImageKind, MediaItem, MediaKind};
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScrapeError {
    #[error("network request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("HTTP {0}")]
    HttpStatus(u16),
    #[error("provider rate limit reached")]
    RateLimited,
    #[error("no result found")]
    NotFound,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("missing API key for {0}")]
    MissingKey(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalId {
    Imdb(String),
    Tmdb(u64),
    TvMaze(u64),
    TvDb(u64),
}

impl ExternalId {
    pub fn from_ids(ids: &ExternalIds) -> Option<Self> {
        ids.imdb
            .clone()
            .map(Self::Imdb)
            .or_else(|| ids.tmdb.map(Self::Tmdb))
            .or_else(|| ids.tvmaze.map(Self::TvMaze))
    }
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub ids: ExternalIds,
    pub language: String,
    pub include_adult: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub external_id: ExternalId,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImageCandidate {
    pub kind: ImageKind,
    pub source_url: String,
    pub language: Option<String>,
    pub season: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DetailSet {
    pub basics: bool,
    pub credits: bool,
    pub images: bool,
}

impl Default for DetailSet {
    fn default() -> Self {
        Self {
            basics: true,
            credits: true,
            images: true,
        }
    }
}

#[async_trait::async_trait]
pub trait MetadataScraper: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn supported_kinds(&self) -> &'static [MediaKind];
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ScrapeError>;
    async fn load(
        &self,
        id: &ExternalId,
        details: &DetailSet,
        language: &str,
    ) -> Result<MediaItem, ScrapeError>;
}

#[async_trait::async_trait]
pub trait ImageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn images(
        &self,
        id: &ExternalId,
        kind: MediaKind,
    ) -> Result<Vec<ImageCandidate>, ScrapeError>;
}

#[derive(Debug, Clone)]
pub struct ScrapeConfig {
    pub tmdb_api_key: Option<String>,
    pub fanarttv_api_key: Option<String>,
    pub fanarttv_client_key: Option<String>,
    pub language: String,
    pub proxy: Option<String>,
    pub cache_ttl: Duration,
    pub timeout: Duration,
    pub backoff: Duration,
}

impl Default for ScrapeConfig {
    fn default() -> Self {
        Self {
            tmdb_api_key: None,
            fanarttv_api_key: None,
            fanarttv_client_key: None,
            language: "en-US".to_owned(),
            proxy: None,
            cache_ttl: Duration::from_secs(240),
            timeout: Duration::from_secs(10),
            backoff: Duration::from_secs(10),
        }
    }
}

struct CacheEntry {
    body: String,
    expires: std::time::Instant,
}

/// Shared HTTP client with a small in-memory response cache and 429 backoff.
pub struct ScrapeClient {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, CacheEntry>>,
    config: ScrapeConfig,
}

impl ScrapeClient {
    pub fn new(config: ScrapeConfig) -> Self {
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("MediaElch/", env!("CARGO_PKG_VERSION")))
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::limited(5));
        if let Some(proxy) = &config.proxy {
            if let Ok(proxy) = reqwest::Proxy::all(proxy) {
                builder = builder.proxy(proxy);
            } else {
                tracing::warn!(%proxy, "invalid proxy configuration ignored");
            }
        }
        let http = builder.build().expect("valid scraper HTTP client");
        Self {
            http,
            cache: Mutex::new(HashMap::new()),
            config,
        }
    }

    pub fn config(&self) -> &ScrapeConfig {
        &self.config
    }

    /// GET a JSON document, using the TTL cache when possible.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, ScrapeError> {
        let body = self.get_text(url).await?;
        serde_json::from_str(&body).map_err(|error| ScrapeError::InvalidResponse(error.to_string()))
    }

    /// GET a text document, using the TTL cache when possible.
    pub async fn get_text(&self, url: &str) -> Result<String, ScrapeError> {
        let key = format!("GET:{url}");
        if let Some(body) = self.cached(&key) {
            return Ok(body);
        }

        let mut attempts = 0;
        loop {
            let response = self.http.get(url).send().await?;
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if attempts >= 2 {
                    return Err(ScrapeError::RateLimited);
                }
                attempts += 1;
                tokio::time::sleep(self.config.backoff).await;
                continue;
            }
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(ScrapeError::NotFound);
            }
            if !status.is_success() {
                return Err(ScrapeError::HttpStatus(status.as_u16()));
            }
            let body = response.text().await?;
            self.cache.lock().unwrap().insert(
                key.clone(),
                CacheEntry {
                    body: body.clone(),
                    expires: std::time::Instant::now() + self.config.cache_ttl,
                },
            );
            return Ok(body);
        }
    }

    /// POST a JSON document and return the response body.
    pub async fn post_json<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T, ScrapeError> {
        let response = self.http.post(url).json(body).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ScrapeError::RateLimited);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ScrapeError::NotFound);
        }
        if !status.is_success() {
            return Err(ScrapeError::HttpStatus(status.as_u16()));
        }
        let text = response.text().await?;
        serde_json::from_str(&text).map_err(|error| ScrapeError::InvalidResponse(error.to_string()))
    }

    fn cached(&self, key: &str) -> Option<String> {
        let cache = self.cache.lock().unwrap();
        cache.get(key).and_then(|entry| {
            if entry.expires > std::time::Instant::now() {
                Some(entry.body.clone())
            } else {
                None
            }
        })
    }
}

pub fn image_url(base: &str, size: &str, path: Option<&str>) -> Option<String> {
    path.filter(|path| !path.is_empty())
        .map(|path| format!("{base}/{size}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_id_from_core_ids_prefers_imdb() {
        let ids = ExternalIds {
            imdb: Some("tt0078748".to_owned()),
            tmdb: Some(348),
            tvmaze: None,
        };
        assert_eq!(
            ExternalId::from_ids(&ids),
            Some(ExternalId::Imdb("tt0078748".to_owned()))
        );
        assert_eq!(
            ExternalId::from_ids(&ExternalIds {
                imdb: None,
                tmdb: Some(348),
                tvmaze: None,
            }),
            Some(ExternalId::Tmdb(348))
        );
    }

    #[test]
    fn image_url_builds_cdn_urls() {
        assert_eq!(
            image_url("https://image.tmdb.org/t/p", "w500", Some("/abc.jpg")),
            Some("https://image.tmdb.org/t/p/w500/abc.jpg".to_owned())
        );
        assert_eq!(image_url("x", "y", None), None);
    }
}
