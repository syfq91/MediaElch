//! Browser-facing HTML routes (server-rendered templates + HTMX).

use std::sync::Arc;

use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use mediaelch_core::{ImageKind, MediaItem, MediaKind};
use mediaelch_db::repository::MediaRepository;
use mediaelch_web::{
    ItemCard, ItemView, LibraryView, ScanJobView, ScanStatusView, ToolsView, APP_CSS,
};

use crate::api::{self, ItemQuery};
use crate::app::AppState;
use crate::write_api::{self, ExportRequest};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ui/library", get(library_page))
        .route("/ui/items/{id}", get(item_page))
        .route("/ui/scan", post(ui_scan))
        .route("/ui/scan/{job_id}", get(ui_scan_status))
        .route("/ui/tools", get(tools_page))
        .route("/ui/export", post(export_form))
        .route("/static/app.css", get(css))
}

pub async fn library_page(
    State(state): State<Arc<AppState>>,
    query: Query<ItemQuery>,
) -> Result<Html<String>, StatusCode> {
    let view = build_library(&state, &query)?;
    view.render().map(Html).map_err(|error| {
        tracing::error!(%error, "rendering library failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

pub async fn item_page(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let mut db = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repo = MediaRepository::new(db.connection_mut());
    let item = repo
        .get(id)
        .map_err(|error| {
            tracing::error!(%error, "loading item failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    ItemView { item }
        .render()
        .map(Html)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn ui_scan(State(state): State<Arc<AppState>>) -> Result<Html<String>, StatusCode> {
    let start = api::start_scan_job(&state);
    let job = state
        .scan_jobs
        .lock()
        .unwrap()
        .get(&start.job_id)
        .cloned()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    render_job(&job)
}

pub async fn ui_scan_status(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let job = state
        .scan_jobs
        .lock()
        .unwrap()
        .get(&job_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    render_job(&job)
}

pub async fn tools_page() -> Result<Html<String>, StatusCode> {
    ToolsView
        .render()
        .map(Html)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn export_form(
    State(state): State<Arc<AppState>>,
    Form(request): Form<ExportRequest>,
) -> Result<Html<String>, StatusCode> {
    let result = write_api::create_export_file(&state, &request)?;
    Ok(Html(format!(
        "<a href=\"/api/export/{}\">Download {}</a> ({} items)",
        result.filename, result.filename, result.items
    )))
}

pub async fn css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS).into_response()
}

fn render_job(job: &api::ScanJob) -> Result<Html<String>, StatusCode> {
    ScanStatusView {
        job: ScanJobView {
            id: job.id.clone(),
            status: job.status.to_owned(),
            processed: job.processed,
            total: job.total,
            error: job.error.clone(),
        },
    }
    .render()
    .map(Html)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn build_library(state: &Arc<AppState>, query: &ItemQuery) -> Result<LibraryView, StatusCode> {
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
    let items = repo
        .list(kind, query.q.as_deref(), 500, 0)
        .map_err(|error| {
            tracing::error!(%error, "listing items failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let total = repo
        .count(kind, query.q.as_deref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(LibraryView {
        items: items.iter().map(card).collect(),
        total,
    })
}

fn card(item: &MediaItem) -> ItemCard {
    let has_poster = item
        .images
        .iter()
        .any(|image| image.kind == ImageKind::Poster && image.local_path.is_file());
    let subtitle = match item.kind {
        MediaKind::Episode => match (item.season_number, item.episode_number) {
            (Some(season), Some(episode)) => format!("S{season:02}E{episode:02}"),
            _ => "Episode".to_owned(),
        },
        _ => item
            .year
            .map(|year| year.to_string())
            .unwrap_or_else(|| item.kind.to_string()),
    };
    ItemCard {
        id: item.id,
        kind: item.kind.to_string(),
        title: item.title.clone(),
        subtitle,
        has_poster,
        poster_url: format!("/api/items/{}/images/poster", item.id),
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

    #[test]
    fn card_uses_episode_numbers_as_subtitle() {
        let mut item = MediaItem::new(
            MediaKind::Episode,
            Default::default(),
            Default::default(),
            "Camp Refoogee".to_owned(),
        );
        item.id = 3;
        item.season_number = Some(2);
        item.episode_number = Some(1);
        let card = card(&item);
        assert_eq!(card.subtitle, "S02E01");
        assert_eq!(card.poster_url, "/api/items/3/images/poster");
    }
}
