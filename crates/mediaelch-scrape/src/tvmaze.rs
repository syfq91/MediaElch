//! TVMaze TV show and episode scraper.

use std::sync::Arc;

use async_trait::async_trait;
use mediaelch_core::{ImageKind, ImageRef, MediaItem, MediaKind, Rating};
use serde_json::Value;

use crate::{
    DetailSet, ExternalId, ImageCandidate, ImageProvider, MetadataScraper, ScrapeClient,
    ScrapeError, SearchHit, SearchQuery,
};

pub const TVMAZE_API_BASE: &str = "https://api.tvmaze.com";

pub struct TvMazeScraper {
    client: Arc<ScrapeClient>,
    base: String,
}

impl TvMazeScraper {
    pub fn new(client: Arc<ScrapeClient>) -> Self {
        Self {
            client,
            base: TVMAZE_API_BASE.to_owned(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ScrapeError> {
        self.client.get_json(&format!("{}{path}", self.base)).await
    }

    /// Load all episodes for a show. TVMaze returns the full list in one call,
    /// which also keeps the API well under its rate limit.
    pub async fn load_episodes(&self, id: &ExternalId) -> Result<Vec<MediaItem>, ScrapeError> {
        let ExternalId::TvMaze(id) = id else {
            return Err(ScrapeError::NotFound);
        };
        let value = self.get(&format!("/shows/{id}/episodes")).await?;
        Ok(parse_episodes(&value))
    }
}

#[async_trait]
impl MetadataScraper for TvMazeScraper {
    fn id(&self) -> &'static str {
        "tvmaze"
    }

    fn name(&self) -> &'static str {
        "TVMaze"
    }

    fn supported_kinds(&self) -> &'static [MediaKind] {
        &[MediaKind::TvShow, MediaKind::Episode]
    }

    async fn load_episodes(&self, id: &ExternalId) -> Result<Vec<MediaItem>, ScrapeError> {
        TvMazeScraper::load_episodes(self, id).await
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ScrapeError> {
        let path = if let Some(imdb) = query.ids.imdb.as_deref() {
            format!("/lookup/shows?imdb={imdb}")
        } else if let Some(text) = query.text.as_deref().filter(|text| !text.is_empty()) {
            format!("/search/shows?q={}", urlencoding::encode(text))
        } else {
            return Ok(Vec::new());
        };
        Ok(parse_search(&self.get(&path).await?))
    }

    async fn load(
        &self,
        kind: MediaKind,
        id: &ExternalId,
        _details: &DetailSet,
        _language: &str,
    ) -> Result<MediaItem, ScrapeError> {
        let ExternalId::TvMaze(id) = id else {
            return Err(ScrapeError::NotFound);
        };
        if kind != MediaKind::TvShow {
            return Err(ScrapeError::NotFound);
        }
        Ok(parse_show(
            &self.get(&format!("/shows/{id}?embed=cast")).await?,
        ))
    }
}

#[async_trait]
impl ImageProvider for TvMazeScraper {
    fn id(&self) -> &'static str {
        "tvmaze"
    }

    async fn images(
        &self,
        id: &ExternalId,
        _kind: MediaKind,
    ) -> Result<Vec<ImageCandidate>, ScrapeError> {
        let ExternalId::TvMaze(id) = id else {
            return Err(ScrapeError::NotFound);
        };
        let value = self.get(&format!("/shows/{id}")).await?;
        let mut images = Vec::new();
        if let Some(image) = value.get("image") {
            if let Some(url) = string(image, "original") {
                images.push(ImageCandidate {
                    kind: ImageKind::Poster,
                    source_url: url,
                    language: None,
                    season: None,
                });
            }
        }
        Ok(images)
    }
}

pub fn parse_search(value: &Value) -> Vec<SearchHit> {
    let Some(results) = value.as_array() else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|entry| {
            let show = entry.get("show")?;
            let id = show.get("id")?.as_u64()?;
            let title = string(show, "name")?;
            Some(SearchHit {
                external_id: ExternalId::TvMaze(id),
                kind: Some(MediaKind::TvShow),
                title,
                year: string(show, "premiered")
                    .and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
                overview: string(show, "summary").map(|summary| strip_html(&summary)),
            })
        })
        .collect()
}

pub fn parse_show(value: &Value) -> MediaItem {
    let mut item = MediaItem::new(
        MediaKind::TvShow,
        Default::default(),
        Default::default(),
        string(value, "name").unwrap_or_default(),
    );
    item.released = string(value, "premiered");
    item.year = item
        .released
        .as_ref()
        .and_then(|date| date.get(..4))
        .and_then(|year| year.parse().ok());
    item.overview = string(value, "summary").map(|summary| strip_html(&summary));
    item.genres = value
        .get("genres")
        .and_then(Value::as_array)
        .map(|genres| {
            genres
                .iter()
                .filter_map(|g| g.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    item.studios = value
        .get("network")
        .and_then(|network| string(network, "name"))
        .into_iter()
        .collect();
    if let Some(rating) = value
        .get("rating")
        .and_then(|rating| rating.get("average"))
        .and_then(Value::as_f64)
    {
        item.ratings.push(Rating {
            source: "tvmaze".to_owned(),
            value: rating,
            votes: 0,
            max: 10.0,
            is_default: true,
        });
    }
    if let Some(externals) = value.get("externals") {
        item.ids.imdb = string(externals, "imdb");
        item.ids.tmdb = string(externals, "themoviedb").and_then(|id| id.parse().ok());
    }
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        item.ids.tvmaze = Some(id);
    }
    if let Some(image) = value.get("image") {
        if let Some(url) = string(image, "original") {
            item.images.push(ImageRef {
                kind: ImageKind::Poster,
                season: None,
                source_url: Some(url),
                local_path: Default::default(),
                language: None,
            });
        }
    }
    item.credits = value
        .get("_embedded")
        .and_then(|embedded| embedded.get("cast"))
        .and_then(Value::as_array)
        .map(|cast| {
            cast.iter()
                .filter_map(|person| {
                    let name = person.get("person")?;
                    Some(mediaelch_core::Credit {
                        kind: mediaelch_core::CreditKind::Actor,
                        name: string(name, "name")?,
                        role: string(person, "character"),
                        thumb_url: name.get("image").and_then(|image| string(image, "medium")),
                        ordering: 0,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    item
}

pub fn parse_episodes(value: &Value) -> Vec<MediaItem> {
    let Some(episodes) = value.as_array() else {
        return Vec::new();
    };
    episodes
        .iter()
        .map(|episode| {
            let mut item = MediaItem::new(
                MediaKind::Episode,
                Default::default(),
                Default::default(),
                string(episode, "name").unwrap_or_default(),
            );
            item.season_number = episode
                .get("season")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            item.episode_number = episode
                .get("number")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            item.released = string(episode, "airdate");
            item.year = item
                .released
                .as_ref()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok());
            item.overview = string(episode, "summary").map(|summary| strip_html(&summary));
            if let Some(image) = episode.get("image") {
                if let Some(url) = string(image, "medium") {
                    item.images.push(ImageRef {
                        kind: ImageKind::EpisodeThumb,
                        season: None,
                        source_url: Some(url),
                        local_path: Default::default(),
                        language: None,
                    });
                }
            }
            item
        })
        .collect()
}

fn strip_html(value: &str) -> String {
    let mut out = String::new();
    let mut inside_tag = false;
    for c in value.chars() {
        match c {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_owned()
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_show_fixture() {
        let item = parse_show(&fixture("tvmaze_show.json"));
        assert_eq!(item.kind, MediaKind::TvShow);
        assert_eq!(item.title, "Under the Dome");
        assert_eq!(item.year, Some(2013));
        assert_eq!(item.genres, vec!["Drama", "Science-Fiction"]);
        assert_eq!(
            item.overview.as_deref(),
            Some("A small town is sealed off.")
        );
        assert_eq!(item.ids.tvmaze, Some(1));
        assert_eq!(item.ids.imdb.as_deref(), Some("tt1553656"));
        assert_eq!(item.ratings[0].value, 7.5);
    }

    #[test]
    fn parses_episodes_fixture() {
        let episodes = parse_episodes(&fixture("tvmaze_episodes.json"));
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].title, "Pilot");
        assert_eq!(episodes[0].season_number, Some(1));
        assert_eq!(episodes[0].episode_number, Some(1));
        assert_eq!(episodes[0].overview.as_deref(), Some("The dome appears."));
        assert_eq!(episodes[1].images.len(), 0);
    }

    #[test]
    fn parses_search_fixture() {
        let hits = parse_search(&fixture("tvmaze_search.json"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Under the Dome");
        assert_eq!(hits[0].external_id, ExternalId::TvMaze(1));
    }
}
