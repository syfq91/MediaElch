//! fanart.tv artwork provider (v3 API).

use std::sync::Arc;

use async_trait::async_trait;
use mediaelch_core::{ImageKind, MediaKind};
use serde_json::Value;

use crate::{ExternalId, ImageCandidate, ImageProvider, ScrapeClient, ScrapeError};

pub const FANART_BASE: &str = "https://webservice.fanart.tv/v3";

pub struct FanartTv {
    client: Arc<ScrapeClient>,
    base: String,
}

impl FanartTv {
    pub fn new(client: Arc<ScrapeClient>) -> Self {
        Self {
            client,
            base: FANART_BASE.to_owned(),
        }
    }

    fn key(&self) -> Result<String, ScrapeError> {
        self.client
            .config()
            .fanarttv_api_key
            .clone()
            .filter(|key| !key.is_empty())
            .ok_or(ScrapeError::MissingKey("fanart.tv"))
    }

    fn url(&self, section: &str, id: u64) -> Result<String, ScrapeError> {
        let mut url = format!("{}/{}/{id}?api_key={}", self.base, section, self.key()?);
        if let Some(client_key) = self
            .client
            .config()
            .fanarttv_client_key
            .as_deref()
            .filter(|key| !key.is_empty())
        {
            url.push_str(&format!("&client_key={client_key}"));
        }
        Ok(url)
    }
}

#[async_trait]
impl ImageProvider for FanartTv {
    fn id(&self) -> &'static str {
        "fanarttv"
    }

    async fn images(
        &self,
        id: &ExternalId,
        kind: MediaKind,
    ) -> Result<Vec<ImageCandidate>, ScrapeError> {
        let numeric_id = match (kind, id) {
            (_, ExternalId::Tmdb(id)) => *id,
            (MediaKind::TvShow, ExternalId::TvDb(id)) => *id,
            (MediaKind::TvShow, ExternalId::TvMaze(id)) => *id,
            _ => return Err(ScrapeError::NotFound),
        };
        let section = match kind {
            MediaKind::TvShow => "tv",
            _ => "movies",
        };
        let value: Value = self
            .client
            .get_json(&self.url(section, numeric_id)?)
            .await?;
        Ok(parse_images(&value, kind))
    }
}

/// Map fanart.tv's category keys to our image kinds.
pub fn parse_images(value: &Value, kind: MediaKind) -> Vec<ImageCandidate> {
    let is_tv = kind == MediaKind::TvShow;
    let groups = [
        (
            if is_tv { "tvposter" } else { "movieposter" },
            ImageKind::Poster,
        ),
        (
            if is_tv {
                "tvbackground"
            } else {
                "moviebackground"
            },
            ImageKind::Fanart,
        ),
        (
            if is_tv { "tvbanner" } else { "moviebanner" },
            ImageKind::Banner,
        ),
        (
            if is_tv { "hdlogos" } else { "hdmovielogo" },
            ImageKind::Logo,
        ),
        (
            if is_tv {
                "hdclearart"
            } else {
                "hdmovieclearart"
            },
            ImageKind::ClearArt,
        ),
    ];

    let mut candidates = Vec::new();
    for (key, image_kind) in groups {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(url) = item.get("url").and_then(Value::as_str) {
                    candidates.push(ImageCandidate {
                        kind: image_kind,
                        source_url: url.to_owned(),
                        language: item
                            .get("lang")
                            .and_then(Value::as_str)
                            .filter(|lang| !lang.is_empty())
                            .map(str::to_owned),
                        season: None,
                    });
                }
            }
        }
    }

    if let Some(seasons) = value.get("seasonposter").and_then(Value::as_object) {
        for (season, items) in seasons {
            let Some(items) = items.as_array() else {
                continue;
            };
            let Some(season) = season.parse::<u32>().ok() else {
                continue;
            };
            for item in items {
                if let Some(url) = item.get("url").and_then(Value::as_str) {
                    candidates.push(ImageCandidate {
                        kind: ImageKind::SeasonPoster,
                        source_url: url.to_owned(),
                        language: item
                            .get("lang")
                            .and_then(Value::as_str)
                            .filter(|lang| !lang.is_empty())
                            .map(str::to_owned),
                        season: Some(season),
                    });
                }
            }
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fanart_fixture() {
        let path = format!(
            "{}/tests/fixtures/fanart_movies.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let images = parse_images(&value, MediaKind::Movie);

        assert_eq!(images.len(), 4);
        assert!(images.iter().any(
            |image| image.kind == ImageKind::Poster && image.language.as_deref() == Some("en")
        ));
        assert!(images.iter().any(|image| image.kind == ImageKind::Fanart));
        assert!(images.iter().any(|image| image.kind == ImageKind::Logo));
        let season = images
            .iter()
            .find(|image| image.kind == ImageKind::SeasonPoster)
            .unwrap();
        assert_eq!(season.season, Some(1));
    }

    #[test]
    fn missing_key_is_reported() {
        let client = Arc::new(ScrapeClient::new(crate::ScrapeConfig::default()));
        let provider = FanartTv::new(client);
        assert!(matches!(
            provider.key(),
            Err(ScrapeError::MissingKey("fanart.tv"))
        ));
    }
}
