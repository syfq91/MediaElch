//! REST API: library browsing, scan jobs and local artwork.

use std::path::Path as StdPath;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mediaelch_core::MediaKind;
use mediaelch_db::repository::MediaRepository;
use mediaelch_fs::{ScanConfig, ScanRoot, Scanner};
use mediaelch_nfo::NfoReader;
use serde::{Deserialize, Serialize};

use crate::app::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct ItemQuery {
    pub kind: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ItemList {
    pub items: Vec<mediaelch_core::MediaItem>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanJob {
    pub id: String,
    pub status: &'static str,
    pub processed: usize,
    pub total: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanStart {
    pub job_id: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/items", get(list_items))
        .route("/api/items/{id}", get(get_item))
        .route("/api/items/{id}/images/{kind}", get(item_image))
        .route("/api/scan", post(start_scan))
        .route("/api/scan/{job_id}", get(scan_status))
}

pub async fn list_items(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ItemQuery>,
) -> Result<Json<ItemList>, StatusCode> {
    let kind = query
        .kind
        .as_deref()
        .map(parse_kind)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut db = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repo = MediaRepository::new(db.connection_mut());
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);

    let items = repo
        .list(kind, query.q.as_deref(), limit, offset)
        .map_err(|error| {
            tracing::error!(%error, "listing items failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let total = repo
        .count(kind, query.q.as_deref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ItemList { items, total }))
}

pub async fn get_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<mediaelch_core::MediaItem>, StatusCode> {
    let mut db = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repo = MediaRepository::new(db.connection_mut());
    repo.get(id)
        .map_err(|error| {
            tracing::error!(%error, "loading item failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub fn start_scan_job(state: &Arc<AppState>) -> ScanStart {
    let job_id = {
        let mut counter = state.job_counter.lock().unwrap();
        *counter += 1;
        format!("scan-{counter}")
    };
    let job = ScanJob {
        id: job_id.clone(),
        status: "queued",
        processed: 0,
        total: 0,
        error: None,
    };
    state.scan_jobs.lock().unwrap().insert(job_id.clone(), job);

    let worker_state = Arc::clone(state);
    let worker_job = job_id.clone();
    tokio::spawn(async move {
        set_job(&worker_state, &worker_job, "running", 0, 0, None);
        let roots = worker_state.roots.clone();
        let scan_state = Arc::clone(&worker_state);
        let scan_job = worker_job.clone();
        let result =
            tokio::task::spawn_blocking(move || run_scan(&scan_state, &roots, &scan_job)).await;
        match result {
            Ok(Ok((processed, total))) => {
                set_job(&worker_state, &worker_job, "done", processed, total, None);
                tracing::info!(processed, total, "library scan finished");
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "library scan failed");
                set_job(&worker_state, &worker_job, "failed", 0, 0, Some(error));
            }
            Err(error) => {
                tracing::error!(%error, "library scan task panicked");
                set_job(
                    &worker_state,
                    &worker_job,
                    "failed",
                    0,
                    0,
                    Some("scan task failed".to_owned()),
                );
            }
        }
    });

    ScanStart { job_id }
}

pub async fn start_scan(State(state): State<Arc<AppState>>) -> Json<ScanStart> {
    Json(start_scan_job(&state))
}

pub async fn scan_status(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<ScanJob>, StatusCode> {
    state
        .scan_jobs
        .lock()
        .unwrap()
        .get(&job_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn item_image(
    State(state): State<Arc<AppState>>,
    Path((id, kind)): Path<(i64, String)>,
) -> Result<Response, StatusCode> {
    let (path, content_type) = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let repo = MediaRepository::new(db.connection_mut());
        let item = repo
            .get(id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let image = item
            .images
            .iter()
            .find(|image| image.kind.as_str() == kind)
            .ok_or(StatusCode::NOT_FOUND)?;
        let content_type = match path_extension(&image.local_path) {
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/jpeg",
        };
        (image.local_path.clone(), content_type)
    };

    if !is_allowed_path(&state, &path) {
        return Err(StatusCode::FORBIDDEN);
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

fn run_scan(
    state: &Arc<AppState>,
    roots: &[ScanRoot],
    job_id: &str,
) -> Result<(usize, usize), String> {
    let scanned = Scanner::new(ScanConfig::default()).scan(roots);
    let total = scanned.items.len();

    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    let mut repo = MediaRepository::new(db.connection_mut());
    let mut episodes = Vec::new();
    let mut processed = 0;

    for mut item in scanned.items {
        if let Some(nfo_path) = item.nfo_path.clone() {
            match NfoReader::read(&nfo_path, item.kind) {
                Ok(draft) => item.apply_nfo(&draft),
                Err(error) => tracing::warn!(%error, "could not read NFO; using filename data"),
            }
        }
        let id = repo.upsert(&item).map_err(|error| error.to_string())?;
        if item.kind == MediaKind::Episode {
            episodes.push((id, item.directory.clone()));
        }
        processed += 1;
        if processed % 10 == 0 {
            set_job(state, job_id, "running", processed, total, None);
        }
    }

    for (episode_id, directory) in episodes {
        if let Some(show_id) = repo
            .find_id(MediaKind::TvShow, &directory)
            .map_err(|error| error.to_string())?
        {
            repo.set_parent(episode_id, show_id)
                .map_err(|error| error.to_string())?;
        }
    }

    for root in roots {
        repo.upsert_root(&root.path, &root.kind, root.recursive)
            .map_err(|error| error.to_string())?;
        repo.touch_root(&root.path)
            .map_err(|error| error.to_string())?;
    }

    Ok((processed, total))
}

fn set_job(
    state: &Arc<AppState>,
    job_id: &str,
    status: &'static str,
    processed: usize,
    total: usize,
    error: Option<String>,
) {
    if let Some(job) = state.scan_jobs.lock().unwrap().get_mut(job_id) {
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

fn path_extension(path: &StdPath) -> &str {
    path.extension().and_then(|ext| ext.to_str()).unwrap_or("")
}

fn is_allowed_path(state: &Arc<AppState>, path: &StdPath) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    state
        .roots
        .iter()
        .map(|root| root.path.as_path())
        .chain(std::iter::once(state.data_dir.as_path()))
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::Request;
    use mediaelch_db::Database;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let db = Database::open_in_memory().unwrap();
        let root = PathBuf::from("/tmp/never-read");
        Arc::new(AppState::new(
            db,
            "test",
            vec![ScanRoot {
                path: root,
                kind: "auto".to_owned(),
                recursive: true,
            }],
            PathBuf::from("/tmp"),
        ))
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn empty_library_lists_zero_items() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/items")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["total"], 0);
        assert_eq!(body["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn unknown_kind_is_rejected() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/items?kind=banana")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_item_is_not_found() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/items/999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn scan_returns_a_job_and_status() {
        let state = test_state();
        let app = crate::app::router(Arc::clone(&state));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scan")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let job_id = body["job_id"].as_str().unwrap().to_owned();

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/scan/{job_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["id"], job_id);
    }

    #[tokio::test]
    async fn unknown_scan_job_is_not_found() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/scan/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
