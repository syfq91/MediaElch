//! HTTP router and shared application state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use mediaelch_db::Database;
use mediaelch_fs::ScanRoot;
use mediaelch_image::ImageConfig;
use serde::{Deserialize, Serialize};

use crate::api::{self, ItemQuery, ScanJob};
use crate::auth::AuthState;
use crate::scrape_api::{ScrapeJob, ScrapeRegistry};
use crate::ui;
use crate::write_api;

/// State shared by every request handler.
pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub version: &'static str,
    pub roots: Vec<ScanRoot>,
    pub data_dir: PathBuf,
    pub scan_jobs: Arc<Mutex<HashMap<String, ScanJob>>>,
    pub job_counter: Arc<Mutex<u64>>,
    pub image_config: ImageConfig,
    pub scrapers: Arc<ScrapeRegistry>,
    pub scrape_jobs: Arc<Mutex<HashMap<String, ScrapeJob>>>,
    pub scrape_counter: Arc<Mutex<u64>>,
    pub auth: Arc<AuthState>,
}

impl AppState {
    pub fn new(
        db: Database,
        version: &'static str,
        roots: Vec<ScanRoot>,
        data_dir: PathBuf,
        image_config: ImageConfig,
        scrapers: Arc<ScrapeRegistry>,
        auth: Arc<AuthState>,
    ) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            version,
            roots,
            data_dir,
            scan_jobs: Arc::new(Mutex::new(HashMap::new())),
            job_counter: Arc::new(Mutex::new(0)),
            image_config,
            scrapers,
            scrape_jobs: Arc::new(Mutex::new(HashMap::new())),
            scrape_counter: Arc::new(Mutex::new(0)),
            auth,
        }
    }
}

/// Response body of `GET /healthz`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
    pub database: String,
}

/// Response body of `GET /api/version`.
#[derive(Debug, Serialize, Deserialize)]
struct VersionInfo {
    name: String,
    version: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    let middleware_state = Arc::clone(&state);
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/api/version", get(version))
        .merge(api::routes())
        .merge(write_api::routes())
        .merge(crate::scrape_api::routes())
        .merge(ui::routes())
        .layer(axum::middleware::from_fn_with_state(
            middleware_state,
            crate::auth::auth_middleware,
        ))
        .merge(crate::auth::routes())
        .with_state(state)
}

async fn root(
    State(state): State<Arc<AppState>>,
) -> Result<axum::response::Html<String>, StatusCode> {
    ui::library_page(State(state), Query(ItemQuery::default())).await
}

async fn healthz(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Health>) {
    let database_ok = match state.db.lock() {
        Ok(db) => db
            .connection()
            .query_row("SELECT 1", [], |_| Ok(()))
            .is_ok(),
        Err(_) => false,
    };

    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = Health {
        status: if database_ok { "ok" } else { "unavailable" }.to_owned(),
        version: state.version.to_owned(),
        database: if database_ok { "ok" } else { "unavailable" }.to_owned(),
    };
    (status, Json(body))
}

async fn version(State(state): State<Arc<AppState>>) -> Json<VersionInfo> {
    Json(VersionInfo {
        name: "MediaElch".to_owned(),
        version: state.version.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router() -> Router {
        let state = AppState::new(
            Database::open_in_memory().unwrap(),
            "test-version",
            Vec::new(),
            PathBuf::from("/tmp"),
            ImageConfig::default(),
            Arc::new(ScrapeRegistry::new(
                mediaelch_scrape::ScrapeConfig::default(),
            )),
            Arc::new(AuthState::new(false, None)),
        );
        router(Arc::new(state))
    }

    async fn body_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn healthz_returns_ok_when_database_is_reachable() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let health: Health = body_json(response).await;
        assert_eq!(health.status, "ok");
        assert_eq!(health.database, "ok");
        assert_eq!(health.version, "test-version");
    }

    #[tokio::test]
    async fn version_endpoint_reports_the_build_version() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let info: VersionInfo = body_json(response).await;
        assert_eq!(info.name, "MediaElch");
        assert_eq!(info.version, "test-version");
    }

    #[tokio::test]
    async fn root_serves_html() {
        let response = test_router()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8(body.to_vec())
            .unwrap()
            .contains("MediaElch"));
    }

    #[tokio::test]
    async fn unknown_route_is_not_found() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
