//! Password authentication and cookie sessions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use password_hash::{rand_core::OsRng, SaltString};
use serde::Deserialize;

use crate::app::AppState;

const COOKIE_NAME: &str = "mediaelch_session";
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct AuthState {
    enabled: bool,
    password_hash: Option<String>,
    sessions: Mutex<HashMap<String, Instant>>,
}

impl AuthState {
    pub fn new(enabled: bool, password_hash: Option<String>) -> Self {
        Self {
            enabled,
            password_hash,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Generate an Argon2id hash for the `--hash-password` CLI flag.
    pub fn hash_password(password: &str) -> Result<String, String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| error.to_string())
    }

    fn verify(&self, password: &str) -> bool {
        let Some(stored) = self.password_hash.as_deref() else {
            return false;
        };
        match PasswordHash::new(stored) {
            Ok(hash) => Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok(),
            Err(error) => {
                tracing::error!(%error, "stored password hash is invalid");
                false
            }
        }
    }

    fn create_session(&self) -> String {
        let token = hex_random();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, created| created.elapsed() < SESSION_TTL);
        sessions.insert(token.clone(), Instant::now());
        token
    }

    fn valid(&self, token: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get(token) {
            Some(created) if created.elapsed() < SESSION_TTL => true,
            Some(_) => {
                sessions.remove(token);
                false
            }
            None => false,
        }
    }

    fn destroy(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.lock().unwrap().remove(token);
        }
    }
}

fn hex_random() -> String {
    use password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cookie_value(header: Option<&str>, name: &str) -> Option<String> {
    header?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_owned())
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/login", get(login_page).post(login))
        .route("/logout", post(logout))
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !state.auth.enabled() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    if path == "/login"
        || path == "/healthz"
        || path == "/api/version"
        || path.starts_with("/static/")
    {
        return next.run(request).await;
    }
    let token = cookie_value(
        request
            .headers()
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
        COOKIE_NAME,
    );
    if token
        .as_deref()
        .is_some_and(|token| state.auth.valid(token))
    {
        return next.run(request).await;
    }
    if path.starts_with("/api/") {
        StatusCode::UNAUTHORIZED.into_response()
    } else {
        Redirect::temporary("/login").into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub password: String,
}

pub async fn login_page() -> Html<&'static str> {
    Html(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Sign in · MediaElch</title><link rel=\"stylesheet\" href=\"/static/app.css\"></head>\
         <body><main class=\"detail\"><h1>MediaElch</h1><h2>Sign in</h2>\
         <form method=\"post\" action=\"/login\">\
         <label>Password <input type=\"password\" name=\"password\" autofocus></label>\
         <button type=\"submit\">Sign in</button></form></main></body></html>",
    )
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Form(form): Form<LoginForm>,
) -> Result<Response, StatusCode> {
    if !state.auth.verify(&form.password) {
        return Ok((StatusCode::UNAUTHORIZED, Html("Invalid password")).into_response());
    }
    let token = state.auth.create_session();
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, "/")]).into_response();
    let cookie = format!("{COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400");
    let value = HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    response.headers_mut().insert(header::SET_COOKIE, value);
    Ok(response)
}

pub async fn logout(State(state): State<Arc<AppState>>, request: Request<Body>) -> Response {
    let token = cookie_value(
        request
            .headers()
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
        COOKIE_NAME,
    );
    state.auth.destroy(token.as_deref());
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, "/login")]).into_response();
    let cleared = format!("{COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if let Ok(value) = HeaderValue::from_str(&cleared) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_passwords() {
        let hash = AuthState::hash_password("correct horse").unwrap();
        assert!(hash.starts_with("$argon2"));
        let state = AuthState::new(true, Some(hash));
        assert!(state.verify("correct horse"));
        assert!(!state.verify("wrong"));
    }

    #[test]
    fn sessions_are_opaque_and_valid() {
        let state = AuthState::new(true, None);
        let token = state.create_session();
        assert_eq!(token.len(), 64);
        assert!(state.valid(&token));
        state.destroy(Some(&token));
        assert!(!state.valid(&token));
    }

    #[tokio::test]
    async fn middleware_protects_api_and_login_sets_a_session() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        use crate::app::AppState;
        use crate::scrape_api::ScrapeRegistry;

        let hash = AuthState::hash_password("secret").unwrap();
        let state = Arc::new(AppState::new(
            mediaelch_db::Database::open_in_memory().unwrap(),
            "test",
            Vec::new(),
            std::path::PathBuf::from("/tmp"),
            mediaelch_image::ImageConfig::default(),
            Arc::new(ScrapeRegistry::new(
                mediaelch_scrape::ScrapeConfig::default(),
            )),
            Arc::new(AuthState::new(true, Some(hash))),
        ));
        let app = crate::app::router(Arc::clone(&state));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/items")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("password=secret"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/items")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn cookie_parsing_ignores_unrelated_cookies() {
        assert_eq!(
            cookie_value(
                Some("other=1; mediaelch_session=abc; theme=dark"),
                COOKIE_NAME
            ),
            Some("abc".to_owned())
        );
        assert_eq!(cookie_value(Some("other=1"), COOKIE_NAME), None);
    }
}
