//! Write-side REST endpoints: NFO save, renamer and exports.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mediaelch_core::MediaKind;
use mediaelch_db::repository::MediaRepository;
use mediaelch_export::{self as exporter, DEFAULT_HTML_TEMPLATE};
use mediaelch_nfo::{KodiVersion, NfoWriter};
use mediaelch_renamer::{self as renamer, RenameEntry};
use serde::{Deserialize, Serialize};

use crate::app::AppState;

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub pattern: String,
    #[serde(default)]
    pub dry_run: bool,
    pub kind: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub format: String,
    pub kind: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NfoSaveResult {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub filename: String,
    pub items: usize,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/items/{id}/nfo", post(save_nfo))
        .route("/api/rename/preview", post(rename_preview))
        .route("/api/rename/apply", post(rename_apply))
        .route("/api/export", post(create_export))
        .route("/api/export/{filename}", get(download_export))
}

pub async fn save_nfo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<NfoSaveResult>, StatusCode> {
    let mut item = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let repo = MediaRepository::new(db.connection_mut());
        repo.get(id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?
    };

    let path = item
        .nfo_path
        .clone()
        .unwrap_or_else(|| item.primary_file.with_extension("nfo"));
    NfoWriter::save(&item, &path, KodiVersion::V21).map_err(|error| {
        tracing::error!(%error, "writing NFO failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    item.nfo_path = Some(path.clone());
    item.nfo_mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());

    {
        let mut db = state
            .db
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut repo = MediaRepository::new(db.connection_mut());
        repo.upsert(&item)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(NfoSaveResult {
        path: path.to_string_lossy().into_owned(),
    }))
}

pub async fn rename_preview(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<Vec<RenameEntry>>, StatusCode> {
    let items = load_items(&state, request.kind.as_deref(), request.q.as_deref())?;
    let entries = items
        .iter()
        .map(|item| match renamer::plan_rename(item, &request.pattern) {
            Ok(entry) => entry,
            Err(error) => RenameEntry {
                item_id: item.id,
                from: item.primary_file.clone(),
                to: item.primary_file.clone(),
                error: Some(error.to_string()),
            },
        })
        .collect();
    Ok(Json(entries))
}

pub async fn rename_apply(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<renamer::ApplySummary>, StatusCode> {
    let items = load_items(&state, request.kind.as_deref(), request.q.as_deref())?;
    let mut entries = Vec::new();
    for item in &items {
        match renamer::plan_rename(item, &request.pattern) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                tracing::warn!(%error, "invalid rename pattern");
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    let summary = renamer::apply(&entries, request.dry_run).map_err(|error| {
        tracing::error!(%error, "rename failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if !request.dry_run {
        let mut db = state
            .db
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let repo = MediaRepository::new(db.connection_mut());
        for entry in &entries {
            if entry.from != entry.to && entry.to.is_file() {
                repo.set_primary_file(entry.item_id, &entry.to)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
        }
    }
    Ok(Json(summary))
}

pub async fn create_export(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ExportRequest>,
) -> Result<Json<ExportResult>, StatusCode> {
    create_export_file(&state, &request).map(Json)
}

pub fn create_export_file(
    state: &Arc<AppState>,
    request: &ExportRequest,
) -> Result<ExportResult, StatusCode> {
    let items = load_items(state, request.kind.as_deref(), request.q.as_deref())?;
    let (extension, content) = match request.format.as_str() {
        "csv" => ("csv", exporter::to_csv(&items, ',')),
        "html" => (
            "html",
            exporter::to_html(&items, DEFAULT_HTML_TEMPLATE)
                .map_err(|_| StatusCode::BAD_REQUEST)?,
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let name = format!(
        "{}-{}.{}",
        request.kind.as_deref().unwrap_or("all"),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        extension
    );
    let dir = state.data_dir.join("exports");
    let path = dir.join(&name);
    exporter::write_file(&path, &content).map_err(|error| {
        tracing::error!(%error, "writing export failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(ExportResult {
        filename: name,
        items: items.len(),
    })
}

pub async fn download_export(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> Result<Response, StatusCode> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = state.data_dir.join("exports").join(&filename);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content_type = if filename.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "text/csv; charset=utf-8"
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

fn load_items(
    state: &Arc<AppState>,
    kind: Option<&str>,
    query: Option<&str>,
) -> Result<Vec<mediaelch_core::MediaItem>, StatusCode> {
    let kind = kind
        .filter(|value| !value.is_empty())
        .map(parse_kind)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut db = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repo = MediaRepository::new(db.connection_mut());
    repo.list(kind, query, 10_000, 0).map_err(|error| {
        tracing::error!(%error, "loading items failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })
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
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::Request;
    use mediaelch_db::Database;
    use mediaelch_image::ImageConfig;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new(
            Database::open_in_memory().unwrap(),
            "test",
            Vec::new(),
            PathBuf::from("/tmp/mediaelch-test"),
            ImageConfig::default(),
            Arc::new(crate::scrape_api::ScrapeRegistry::new(
                mediaelch_scrape::ScrapeConfig::default(),
            )),
        ))
    }

    #[tokio::test]
    async fn rename_preview_on_empty_library_is_empty() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/rename/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"pattern":"<title>.<extension>"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn export_rejects_unknown_format() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/export")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"format":"pdf"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn export_download_rejects_path_traversal() {
        let response = crate::app::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/export/..%2Fsecret.csv")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            response.status() == StatusCode::BAD_REQUEST
                || response.status() == StatusCode::NOT_FOUND
        );
    }
}
