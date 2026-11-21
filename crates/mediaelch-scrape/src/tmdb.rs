//! TMDB movie, TV and concert scraper (API v3).

use std::sync::Arc;

use async_trait::async_trait;
use mediaelch_core::{
    Credit, CreditKind, ExternalIds, ImageKind, ImageRef, MediaItem, MediaKind, Rating,
};
use serde_json::Value;

use crate::{
    image_url, DetailSet, ExternalId, ImageCandidate, ImageProvider, MetadataScraper, ScrapeClient,
    ScrapeError, SearchHit, SearchQuery,
};

pub const TMDB_API_BASE: &str = "https://api.themoviedb.org/3";
pub const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p";

pub struct TmdbScraper {
    client: Arc<ScrapeClient>,
    api_base: String,
    image_base: String,
}

impl TmdbScraper {
    pub fn new(client: Arc<ScrapeClient>) -> Self {
        Self {
            client,
            api_base: TMDB_API_BASE.to_owned(),
            image_base: TMDB_IMAGE_BASE.to_owned(),
        }
    }

    #[cfg(test)]
    fn with_bases(client: Arc<ScrapeClient>, api_base: &str, image_base: &str) -> Self {
        Self {
            client,
            api_base: api_base.to_owned(),
            image_base: image_base.to_owned(),
        }
    }

    fn key(&self) -> Result<String, ScrapeError> {
        self.client
            .config()
            .tmdb_api_key
            .clone()
            .filter(|key| !key.is_empty())
            .ok_or(ScrapeError::MissingKey("TMDB"))
    }

    fn url(&self, path: &str, key: &str, extra: &str) -> String {
        let separator = if path.contains('?') { '&' } else { '?' };
        format!("{}{path}{separator}api_key={key}{extra}", self.api_base)
    }

    async fn load_by_path(
        &self,
        path: &str,
        extra: &str,
        kind: MediaKind,
        details: &DetailSet,
        language: &str,
    ) -> Result<MediaItem, ScrapeError> {
        let key = self.key()?;
        let append = if details.credits || details.images {
            let mut parts = Vec::new();
            if details.credits {
                parts.push("credits");
            }
            if details.images {
                parts.push("images");
            }
            format!("&append_to_response={}", parts.join(","))
        } else {
            String::new()
        };
        let url = self.url(path, &key, &format!("&language={language}{append}{extra}"));
        let value: Value = self.client.get_json(&url).await?;
        Ok(match kind {
            MediaKind::TvShow => parse_tv(&value, &self.image_base),
            _ => parse_movie(&value, &self.image_base),
        })
    }
}

#[async_trait]
impl MetadataScraper for TmdbScraper {
    fn id(&self) -> &'static str {
        "tmdb"
    }

    fn name(&self) -> &'static str {
        "The Movie Database"
    }

    fn supported_kinds(&self) -> &'static [MediaKind] {
        &[MediaKind::Movie, MediaKind::TvShow, MediaKind::Concert]
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ScrapeError> {
        let key = self.key()?;
        let language = &query.language;
        let mut hits = Vec::new();

        if let Some(ExternalId::Imdb(imdb)) = ExternalId::from_ids(&query.ids) {
            let url = self.url(
                &format!("/find/{imdb}"),
                &key,
                &format!("&external_source=imdb_id&language={language}"),
            );
            let value: Value = self.client.get_json(&url).await?;
            hits.extend(parse_search(&value));
        } else if let Some(text) = query.text.as_deref().filter(|text| !text.is_empty()) {
            let encoded = urlencoding::encode(text);
            for path in ["/search/movie", "/search/tv"] {
                let url = self.url(
                    path,
                    &key,
                    &format!(
                        "&query={encoded}&language={language}&include_adult={}",
                        query.include_adult
                    ),
                );
                let value: Value = self.client.get_json(&url).await?;
                hits.extend(parse_search(&value));
            }
        }
        Ok(hits)
    }

    async fn load(
        &self,
        kind: MediaKind,
        id: &ExternalId,
        details: &DetailSet,
        language: &str,
    ) -> Result<MediaItem, ScrapeError> {
        let tmdb_id = match id {
            ExternalId::Tmdb(id) => *id,
            ExternalId::Imdb(imdb) => {
                let hits = self
                    .search(&SearchQuery {
                        text: None,
                        ids: ExternalIds {
                            imdb: Some(imdb.clone()),
                            ..ExternalIds::default()
                        },
                        language: language.to_owned(),
                        include_adult: false,
                    })
                    .await?;
                match hits.first().map(|hit| &hit.external_id) {
                    Some(ExternalId::Tmdb(id)) => *id,
                    _ => return Err(ScrapeError::NotFound),
                }
            }
            _ => return Err(ScrapeError::NotFound),
        };
        let path = match kind {
            MediaKind::TvShow => format!("/tv/{tmdb_id}"),
            _ => format!("/movie/{tmdb_id}"),
        };
        self.load_by_path(&path, "", kind, details, language).await
    }
}

#[async_trait]
impl ImageProvider for TmdbScraper {
    fn id(&self) -> &'static str {
        "tmdb"
    }

    async fn images(
        &self,
        id: &ExternalId,
        kind: MediaKind,
    ) -> Result<Vec<ImageCandidate>, ScrapeError> {
        let tmdb_id = match id {
            ExternalId::Tmdb(id) => *id,
            _ => return Err(ScrapeError::NotFound),
        };
        let key = self.key()?;
        let path = match kind {
            MediaKind::TvShow => format!("/tv/{tmdb_id}/images"),
            _ => format!("/movie/{tmdb_id}/images"),
        };
        let url = self.url(&path, &key, "");
        let value: Value = self.client.get_json(&url).await?;
        Ok(parse_images(&value, &self.image_base))
    }
}

pub fn parse_search(value: &Value) -> Vec<SearchHit> {
    let mut all = Vec::new();
    for key in ["results", "movie_results", "tv_results"] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            all.extend(items.iter());
        }
    }
    all.into_iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_u64()?;
            let title = item
                .get("title")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?
                .to_owned();
            let year = item
                .get("release_date")
                .or_else(|| item.get("first_air_date"))
                .and_then(Value::as_str)
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok());
            let kind = match item.get("media_type").and_then(Value::as_str) {
                Some("tv") => Some(MediaKind::TvShow),
                Some("movie") => Some(MediaKind::Movie),
                _ => None,
            };
            Some(SearchHit {
                external_id: ExternalId::Tmdb(id),
                kind,
                title,
                year,
                overview: item
                    .get("overview")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub fn parse_movie(value: &Value, image_base: &str) -> MediaItem {
    let mut item = MediaItem::new(
        MediaKind::Movie,
        Default::default(),
        Default::default(),
        string(value, "title").unwrap_or_default(),
    );
    item.original_title = string(value, "original_title");
    item.overview = string(value, "overview");
    item.year = string(value, "release_date")
        .and_then(|date| date.get(..4).and_then(|year| year.parse().ok()));
    item.released = string(value, "release_date");
    item.runtime_min = value
        .get("runtime")
        .and_then(Value::as_i64)
        .map(|n| n as i32);
    item.genres = string_array(value, "genres", "name");
    item.countries = string_array(value, "production_countries", "name");
    item.studios = string_array(value, "production_companies", "name");

    if let Some(rating) = value.get("vote_average").and_then(Value::as_f64) {
        item.ratings.push(Rating {
            source: "themoviedb".to_owned(),
            value: rating,
            votes: value.get("vote_count").and_then(Value::as_u64).unwrap_or(0),
            max: 10.0,
            is_default: true,
        });
    }
    if let Some(certification) = value
        .get("release_dates")
        .and_then(|v| v.get("results"))
        .and_then(Value::as_array)
        .and_then(|results| {
            results
                .iter()
                .find(|r| string(r, "iso_3166_1").as_deref() == Some("US"))
        })
        .and_then(|r| r.get("release_dates"))
        .and_then(Value::as_array)
        .and_then(|dates| dates.first())
        .and_then(|date| string(date, "certification"))
    {
        item.certification = Some(certification);
    }

    if let Some(ids) = value.get("external_ids") {
        item.ids.imdb = string(ids, "imdb_id");
    }
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        item.ids.tmdb = Some(id);
    }

    if let Some(cast) = value
        .get("credits")
        .and_then(|v| v.get("cast"))
        .and_then(Value::as_array)
    {
        for (index, person) in cast.iter().enumerate() {
            let Some(name) = string(person, "name") else {
                continue;
            };
            let thumb = string(person, "profile_path")
                .and_then(|path| image_url(image_base, "w185", Some(&path)));
            item.credits.push(Credit {
                kind: CreditKind::Actor,
                name: name.clone(),
                role: string(person, "character"),
                thumb_url: thumb.clone(),
                ordering: person
                    .get("order")
                    .and_then(Value::as_u64)
                    .unwrap_or(index as u64) as u32,
            });
            if let Some(url) = thumb {
                item.images.push(ImageRef {
                    kind: ImageKind::Actor,
                    season: None,
                    source_url: Some(url),
                    local_path: Default::default(),
                    language: None,
                });
            }
        }
    }
    if let Some(crew) = value
        .get("credits")
        .and_then(|v| v.get("crew"))
        .and_then(Value::as_array)
    {
        for person in crew
            .iter()
            .filter(|p| string(p, "job").as_deref() == Some("Director"))
        {
            if let Some(name) = string(person, "name") {
                item.credits.push(Credit {
                    kind: CreditKind::Director,
                    name,
                    role: None,
                    thumb_url: None,
                    ordering: 0,
                });
            }
        }
    }

    item.images.extend(
        parse_images(value, image_base)
            .into_iter()
            .map(|candidate| ImageRef {
                kind: candidate.kind,
                season: candidate.season,
                source_url: Some(candidate.source_url),
                local_path: Default::default(),
                language: candidate.language,
            }),
    );
    item
}

pub fn parse_tv(value: &Value, image_base: &str) -> MediaItem {
    let mut item = parse_movie(value, image_base);
    item.kind = MediaKind::TvShow;
    item.title = string(value, "name").unwrap_or_default();
    item.original_title = string(value, "original_name");
    item.released = string(value, "first_air_date");
    item.year = item
        .released
        .as_ref()
        .and_then(|date| date.get(..4))
        .and_then(|year| year.parse().ok());
    item.runtime_min = value
        .get("episode_run_time")
        .and_then(Value::as_array)
        .and_then(|times| times.first())
        .and_then(Value::as_i64)
        .map(|n| n as i32);
    if let Some(ratings) = value
        .get("content_ratings")
        .and_then(|v| v.get("results"))
        .and_then(Value::as_array)
        .and_then(|results| {
            results
                .iter()
                .find(|r| string(r, "iso_3166_1").as_deref() == Some("US"))
        })
    {
        item.certification = string(ratings, "rating");
    }
    item
}

pub fn parse_images(value: &Value, image_base: &str) -> Vec<ImageCandidate> {
    let mut candidates = Vec::new();
    for (key, kind) in [
        ("posters", ImageKind::Poster),
        ("backdrops", ImageKind::Fanart),
    ] {
        let list = value
            .get(key)
            .or_else(|| value.get("images").and_then(|images| images.get(key)));
        if let Some(images) = list.and_then(Value::as_array) {
            for image in images {
                let Some(path) = string(image, "file_path") else {
                    continue;
                };
                let size = if kind == ImageKind::Poster {
                    "w500"
                } else {
                    "original"
                };
                let Some(url) = image_url(image_base, size, Some(&path)) else {
                    continue;
                };
                candidates.push(ImageCandidate {
                    kind,
                    source_url: url,
                    language: string(image, "iso_639_1"),
                    season: None,
                });
            }
        }
    }
    candidates
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn string_array(value: &Value, array: &str, field: &str) -> Vec<String> {
    value
        .get(array)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| string(item, field))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScrapeConfig;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn parses_movie_fixture() {
        let item = parse_movie(&fixture("tmdb_movie.json"), TMDB_IMAGE_BASE);
        assert_eq!(item.title, "Alien");
        assert_eq!(item.year, Some(1979));
        assert_eq!(item.runtime_min, Some(117));
        assert_eq!(item.ids.imdb.as_deref(), Some("tt0078748"));
        assert_eq!(item.ids.tmdb, Some(348));
        assert_eq!(item.genres, vec!["Horror", "Science Fiction"]);
        assert_eq!(item.certification.as_deref(), Some("R"));
        assert_eq!(item.ratings[0].votes, 7653);
        assert_eq!(item.credits[0].name, "Sigourney Weaver");
        assert!(item
            .images
            .iter()
            .any(|image| image.source_url.as_deref()
                == Some("https://image.tmdb.org/t/p/w500/abc.jpg")));
    }

    #[test]
    fn parses_tv_fixture() {
        let item = parse_tv(&fixture("tmdb_tv.json"), TMDB_IMAGE_BASE);
        assert_eq!(item.kind, MediaKind::TvShow);
        assert_eq!(item.title, "Breaking Bad");
        assert_eq!(item.year, Some(2008));
        assert_eq!(item.runtime_min, Some(49));
        assert_eq!(item.certification.as_deref(), Some("TV-MA"));
        assert_eq!(item.ids.tvmaze, None);
    }

    #[test]
    fn parses_search_results() {
        let hits = parse_search(&fixture("tmdb_search.json"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Alien");
        assert_eq!(hits[0].year, Some(1979));
        assert_eq!(hits[1].kind, Some(MediaKind::TvShow));
    }

    #[test]
    fn missing_api_key_is_reported() {
        let client = Arc::new(ScrapeClient::new(ScrapeConfig {
            tmdb_api_key: None,
            backoff: std::time::Duration::ZERO,
            ..ScrapeConfig::default()
        }));
        assert!(matches!(
            TmdbScraper::with_bases(client, "http://localhost", "http://localhost").key(),
            Err(ScrapeError::MissingKey("TMDB"))
        ));
    }
}
