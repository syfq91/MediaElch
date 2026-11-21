//! Scraper registry, search/scrape endpoints and SSE progress.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Sse;
use axum::routing::{get, post};
use axum::{Json, Router};
use mediaelch_core::{ExternalIds, ImageRef, MediaItem, MediaKind};
use mediaelch_db::repository::MediaRepository;
use mediaelch_scrape::{
    DetailSet, ExternalId, FanartTv, ImageProvider, MetadataScraper, ScrapeClient, ScrapeConfig,
    SearchQuery, TmdbScraper, TvMazeScraper,
};
use serde::{Deserialize, Serialize};

use crate::app::AppState;

#[derive(Debug, Serialize)]
pub struct ScraperInfo {
    pub id: String,
    pub name: String,
    pub kinds: Vec<&'static str>,
}

pub struct ScrapeRegistry {
    metadata: HashMap<String, Arc<dyn MetadataScraper>>,
    images: HashMap<String, Arc<dyn ImageProvider>>,
    client: Arc<ScrapeClient>,
}

impl ScrapeRegistry {
    pub fn new(config: ScrapeConfig) -> Self {
        let client = Arc::new(ScrapeClient::new(config));
        let mut metadata: HashMap<String, Arc<dyn MetadataScraper>> = HashMap::new();
        let mut images: HashMap<String, Arc<dyn ImageProvider>> = HashMap::new();

        let tmdb = Arc::new(TmdbScraper::new(Arc::clone(&client)));
        metadata.insert("tmdb".to_owned(), tmdb.clone());
        images.insert("tmdb".to_owned(), tmdb);

        let tvmaze = Arc::new(TvMazeScraper::new(Arc::clone(&client)));
        metadata.insert("tvmaze".to_owned(), tvmaze.clone());
        images.insert("tvmaze".to_owned(), tvmaze);

        images.insert(
            "fanarttv".to_owned(),
            Arc::new(FanartTv::new(Arc::clone(&client))),
        );

        Self {
            metadata,
            images,
            client,
        }
    }

    pub fn list(&self) -> Vec<ScraperInfo> {
        let mut info: Vec<ScraperInfo> = self
            .metadata
            .values()
            .map(|scraper| ScraperInfo {
                id: scraper.id().to_owned(),
                name: scraper.name().to_owned(),
                kinds: scraper
                    .supported_kinds()
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect(),
            })
            .collect();
        for provider in self.images.values() {
            if !info.iter().any(|entry| entry.id == provider.id()) {
                info.push(ScraperInfo {
                    id: provider.id().to_owned(),
                    name: provider.id().to_owned(),
                    kinds: Vec::new(),
                });
            }
        }
        info.sort_by(|a, b| a.id.cmp(&b.id));
        info
    }

    pub fn metadata(&self, id: &str) -> Option<Arc<dyn MetadataScraper>> {
        self.metadata.get(id).map(Arc::clone)
    }

    pub fn images(&self, id: &str) -> Option<Arc<dyn ImageProvider>> {
        self.images.get(id).map(Arc::clone)
    }

    pub fn config(&self) -> &ScrapeConfig {
        self.client.config()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScrapeJob {
    pub id: String,
    pub status: &'static str,
    pub item_id: i64,
    pub processed: usize,
    pub total: usize,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScrapeSearchQuery {
    pub provider: String,
    pub kind: Option<String>,
    pub q: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeRequest {
    pub item_id: i64,
    pub provider: String,
    pub image_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddItemRequest {
    pub kind: String,
    pub title: String,
    pub year: Option<i32>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<u64>,
    pub tvmaze_id: Option<u64>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/scrapers", get(list_scrapers))
        .route("/api/scrape/search", get(search))
        .route("/api/scrape", post(start_scrape))
        .route("/api/scrape/{job_id}", get(scrape_status))
        .route("/api/scrape/events", get(scrape_events))
        .route("/api/items/add", post(add_item))
}

pub async fn list_scrapers(State(state): State<Arc<AppState>>) -> Json<Vec<ScraperInfo>> {
    Json(state.scrapers.list())
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ScrapeSearchQuery>,
) -> Result<Json<Vec<mediaelch_scrape::SearchHit>>, StatusCode> {
    let scraper = state
        .scrapers
        .metadata(&query.provider)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if let Some(kind) = query.kind.as_deref().filter(|kind| !kind.is_empty()) {
        let kind = parse_kind(kind).map_err(|_| StatusCode::BAD_REQUEST)?;
        if !scraper.supported_kinds().contains(&kind) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let hits = scraper
        .search(&SearchQuery {
            text: Some(query.q),
            ids: ExternalIds::default(),
            language: state.scrapers.config().language.clone(),
            include_adult: false,
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "scraper search failed");
            StatusCode::BAD_GATEWAY
        })?;
    Ok(Json(hits))
}

pub async fn start_scrape(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScrapeRequest>,
) -> Result<Json<ScrapeJob>, StatusCode> {
    start_scrape_job(&state, &request).map(Json)
}

pub fn start_scrape_job(
    state: &Arc<AppState>,
    request: &ScrapeRequest,
) -> Result<ScrapeJob, StatusCode> {
    if state.scrapers.metadata(&request.provider).is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let job_id = {
        let mut counter = state.scrape_counter.lock().unwrap();
        *counter += 1;
        format!("scrape-{counter}")
    };
    let job = ScrapeJob {
        id: job_id.clone(),
        status: "queued",
        item_id: request.item_id,
        processed: 0,
        total: 0,
        error: None,
    };
    state
        .scrape_jobs
        .lock()
        .unwrap()
        .insert(job_id.clone(), job.clone());

    let worker_state = Arc::clone(state);
    let worker_job = job_id.clone();
    let worker_request = request.clone();
    tokio::spawn(async move {
        set_status(&worker_state, &worker_job, "running", 0, 0, None);
        match run_scrape(&worker_state, &worker_request, &worker_job).await {
            Ok((processed, total)) => {
                set_status(&worker_state, &worker_job, "done", processed, total, None);
            }
            Err(error) => {
                tracing::error!(%error, "scrape job failed");
                set_status(&worker_state, &worker_job, "failed", 0, 0, Some(error));
            }
        }
    });
    Ok(job)
}

pub async fn scrape_status(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<ScrapeJob>, StatusCode> {
    state
        .scrape_jobs
        .lock()
        .unwrap()
        .get(&job_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn scrape_events(
    State(state): State<Arc<AppState>>,
) -> Sse<
    impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let stream = async_stream::stream! {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let jobs: Vec<ScrapeJob> = {
                let jobs = state.scrape_jobs.lock().unwrap();
                jobs.values().cloned().collect()
            };
            let data = serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".to_owned());
            yield Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().event("jobs").data(data)
            );
        }
    };
    Sse::new(stream)
}

pub async fn add_item(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddItemRequest>,
) -> Result<Json<mediaelch_core::MediaItem>, StatusCode> {
    let kind = parse_kind(&request.kind).map_err(|_| StatusCode::BAD_REQUEST)?;
    let slug: String = request
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let directory = state.data_dir.join("imported").join(slug);
    let primary_file = directory.join(format!("{}.nfo", request.title));
    let mut item = MediaItem::new(kind, directory, primary_file, request.title);
    item.year = request.year;
    item.ids = ExternalIds {
        imdb: request.imdb_id,
        tmdb: request.tmdb_id,
        tvmaze: request.tvmaze_id,
    };
    let mut db = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut repo = MediaRepository::new(db.connection_mut());
    let id = repo
        .upsert(&item)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    item.id = id;
    Ok(Json(item))
}

async fn run_scrape(
    state: &Arc<AppState>,
    request: &ScrapeRequest,
    job_id: &str,
) -> Result<(usize, usize), String> {
    let item = {
        let mut db = state.db.lock().map_err(|error| error.to_string())?;
        let repo = MediaRepository::new(db.connection_mut());
        repo.get(request.item_id)
            .map_err(|error| error.to_string())?
            .ok_or("item not found")?
    };
    let scraper = state
        .scrapers
        .metadata(&request.provider)
        .ok_or("unknown scraper")?;
    let external_id = resolve_id(&*scraper, &item).await?;
    let language = state.scrapers.config().language.clone();

    let mut draft = scraper
        .load(item.kind, &external_id, &DetailSet::default(), &language)
        .await
        .map_err(|error| error.to_string())?;
    let episodes = scraper.load_episodes(&external_id).await.ok();
    set_status(state, job_id, "running", 1, 3, None);

    if let Some(image_provider_id) = request.image_provider.as_deref() {
        if let Some(provider) = state.scrapers.images(image_provider_id) {
            match provider.images(&external_id, item.kind).await {
                Ok(candidates) => {
                    for candidate in candidates {
                        draft.images.push(ImageRef {
                            kind: candidate.kind,
                            season: candidate.season,
                            source_url: Some(candidate.source_url),
                            local_path: std::path::PathBuf::new(),
                            language: candidate.language,
                        });
                    }
                }
                Err(error) => tracing::warn!(%error, "image provider failed"),
            }
        }
    }
    set_status(state, job_id, "running", 2, 3, None);

    let mut item = item;
    item.apply_nfo(&draft);

    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    let mut repo = MediaRepository::new(db.connection_mut());
    repo.upsert(&item).map_err(|error| error.to_string())?;

    if let Some(episodes) = episodes {
        let show_id = repo
            .find_id(MediaKind::TvShow, &item.directory)
            .map_err(|error| error.to_string())?
            .unwrap_or(item.id);
        for mut episode in episodes {
            episode.directory = item.directory.clone();
            let id = repo.upsert(&episode).map_err(|error| error.to_string())?;
            repo.set_parent(id, show_id)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok((3, 3))
}

async fn resolve_id(scraper: &dyn MetadataScraper, item: &MediaItem) -> Result<ExternalId, String> {
    match scraper.id() {
        "tmdb" => match (item.ids.tmdb, item.ids.imdb.as_deref()) {
            (Some(id), _) => Ok(ExternalId::Tmdb(id)),
            (None, Some(imdb)) => Ok(ExternalId::Imdb(imdb.to_owned())),
            _ => Err("item has no TMDB or IMDb id".to_owned()),
        },
        "tvmaze" => match item.ids.tvmaze {
            Some(id) => Ok(ExternalId::TvMaze(id)),
            None => {
                let ids = ExternalIds {
                    imdb: item.ids.imdb.clone(),
                    ..ExternalIds::default()
                };
                if ids.imdb.is_none() {
                    return Err("item has no TVMaze or IMDb id".to_owned());
                }
                let hits = scraper
                    .search(&SearchQuery {
                        text: None,
                        ids,
                        language: "en-US".to_owned(),
                        include_adult: false,
                    })
                    .await
                    .map_err(|error| error.to_string())?;
                hits.first()
                    .map(|hit| hit.external_id.clone())
                    .ok_or_else(|| "no TVMaze match found".to_owned())
            }
        },
        _ => Err("unsupported scraper".to_owned()),
    }
}

fn set_status(
    state: &Arc<AppState>,
    job_id: &str,
    status: &'static str,
    processed: usize,
    total: usize,
    error: Option<String>,
) {
    if let Some(job) = state.scrape_jobs.lock().unwrap().get_mut(job_id) {
        job.status = status;
        job.processed = processed;
        job.total = total;
        job.error = error;
    }
}

fn parse_kind(value: &str) -> Result<MediaKind, ()> {
    match value {
        "movie" => Ok(MediaKind::Movie),
        "tvshow" => Ok(MediaKind::TvShow),
        "episode" => Ok(MediaKind::Episode),
        "concert" => Ok(MediaKind::Concert),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ScrapeRegistry {
        ScrapeRegistry::new(ScrapeConfig {
            backoff: Duration::ZERO,
            ..ScrapeConfig::default()
        })
    }

    #[test]
    fn registry_exposes_all_providers() {
        let info = registry().list();
        let ids: Vec<_> = info.iter().map(|entry| entry.id.as_str()).collect();
        assert!(ids.contains(&"tmdb"));
        assert!(ids.contains(&"tvmaze"));
        assert!(ids.contains(&"fanarttv"));
    }

    #[test]
    fn missing_api_key_does_not_prevent_registry_creation() {
        let registry = registry();
        assert!(registry.metadata("tmdb").is_some());
        assert!(registry.images("fanarttv").is_some());
    }

    #[test]
    fn unknown_provider_is_not_found() {
        let registry = registry();
        assert!(registry.metadata("nope").is_none());
        assert!(registry.images("nope").is_none());
    }
}
