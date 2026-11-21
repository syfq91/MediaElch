//! Domain model for the headless MediaElch server.
//!
//! Pure data types only: no I/O, no framework dependencies, no Qt.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The kinds of media items the library manager handles in the first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Movie,
    TvShow,
    Episode,
    Concert,
}

impl MediaKind {
    /// Stable lowercase name used in the database and REST API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::TvShow => "tvshow",
            Self::Episode => "episode",
            Self::Concert => "concert",
        }
    }
}

impl std::fmt::Display for MediaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditKind {
    Actor,
    Director,
    Writer,
}

impl CreditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Director => "director",
            Self::Writer => "writer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Poster,
    Fanart,
    Banner,
    SeasonPoster,
    EpisodeThumb,
    Actor,
    Logo,
    ClearArt,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Fanart => "fanart",
            Self::Banner => "banner",
            Self::SeasonPoster => "season",
            Self::EpisodeThumb => "episode_thumb",
            Self::Actor => "actor",
            Self::Logo => "logo",
            Self::ClearArt => "clearart",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExternalIds {
    pub imdb: Option<String>,
    pub tmdb: Option<u64>,
    pub tvmaze: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub source: String,
    pub value: f64,
    pub votes: u64,
    pub max: f64,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credit {
    pub kind: CreditKind,
    pub name: String,
    pub role: Option<String>,
    pub thumb_url: Option<String>,
    pub ordering: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub kind: ImageKind,
    pub season: Option<u32>,
    pub source_url: Option<String>,
    pub local_path: PathBuf,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFileRef {
    pub path: PathBuf,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
    pub stack_part: Option<u32>,
    pub is_extra: bool,
}

/// A media item as seen by the scanner, NFO reader, database and API.
/// An item that has not been persisted uses `id == 0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: i64,
    pub kind: MediaKind,
    pub parent_id: Option<i64>,
    pub directory: PathBuf,
    pub primary_file: PathBuf,
    pub title: String,
    pub sort_title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub released: Option<String>,
    pub overview: Option<String>,
    pub runtime_min: Option<i32>,
    pub certification: Option<String>,
    pub ratings: Vec<Rating>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub studios: Vec<String>,
    pub countries: Vec<String>,
    pub ids: ExternalIds,
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    pub nfo_path: Option<PathBuf>,
    pub nfo_mtime: Option<u64>,
    pub credits: Vec<Credit>,
    pub images: Vec<ImageRef>,
    pub files: Vec<MediaFileRef>,
}

impl MediaItem {
    /// Create a minimal item from scanner output.
    pub fn new(kind: MediaKind, directory: PathBuf, primary_file: PathBuf, title: String) -> Self {
        Self {
            id: 0,
            kind,
            parent_id: None,
            directory,
            primary_file,
            title,
            sort_title: None,
            original_title: None,
            year: None,
            released: None,
            overview: None,
            runtime_min: None,
            certification: None,
            ratings: Vec::new(),
            genres: Vec::new(),
            tags: Vec::new(),
            studios: Vec::new(),
            countries: Vec::new(),
            ids: ExternalIds::default(),
            season_number: None,
            episode_number: None,
            nfo_path: None,
            nfo_mtime: None,
            credits: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Overlay values parsed from an NFO file. NFO values win over scanner
    /// guesses; empty fields in the NFO leave the scanned value untouched.
    pub fn apply_nfo(&mut self, nfo: &Self) {
        if !nfo.title.is_empty() {
            self.title = nfo.title.clone();
        }
        take_if_some(&mut self.sort_title, &nfo.sort_title);
        take_if_some(&mut self.original_title, &nfo.original_title);
        take_if_some(&mut self.year, &nfo.year);
        take_if_some(&mut self.released, &nfo.released);
        take_if_some(&mut self.overview, &nfo.overview);
        take_if_some(&mut self.runtime_min, &nfo.runtime_min);
        take_if_some(&mut self.certification, &nfo.certification);
        take_if_some(&mut self.season_number, &nfo.season_number);
        take_if_some(&mut self.episode_number, &nfo.episode_number);
        take_if_some(&mut self.nfo_path, &nfo.nfo_path);
        take_if_some(&mut self.nfo_mtime, &nfo.nfo_mtime);

        replace_if_not_empty(&mut self.genres, &nfo.genres);
        replace_if_not_empty(&mut self.tags, &nfo.tags);
        replace_if_not_empty(&mut self.studios, &nfo.studios);
        replace_if_not_empty(&mut self.countries, &nfo.countries);
        replace_if_not_empty(&mut self.ratings, &nfo.ratings);
        replace_if_not_empty(&mut self.credits, &nfo.credits);
        replace_if_not_empty(&mut self.images, &nfo.images);

        if nfo.ids.imdb.is_some() {
            self.ids.imdb = nfo.ids.imdb.clone();
        }
        if nfo.ids.tmdb.is_some() {
            self.ids.tmdb = nfo.ids.tmdb;
        }
        if nfo.ids.tvmaze.is_some() {
            self.ids.tvmaze = nfo.ids.tvmaze;
        }
    }

    /// The rating that should be displayed first: the flagged default, or the
    /// first one listed.
    pub fn primary_rating(&self) -> Option<&Rating> {
        self.ratings
            .iter()
            .find(|r| r.is_default)
            .or(self.ratings.first())
    }
}

fn take_if_some<T>(target: &mut Option<T>, source: &Option<T>)
where
    T: Clone,
{
    if source.is_some() {
        *target = source.clone();
    }
}

fn replace_if_not_empty<T>(target: &mut Vec<T>, source: &[T])
where
    T: Clone,
{
    if !source.is_empty() {
        *target = source.to_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_kind_as_str_matches_database_values() {
        assert_eq!(MediaKind::Movie.as_str(), "movie");
        assert_eq!(MediaKind::TvShow.as_str(), "tvshow");
        assert_eq!(MediaKind::Episode.as_str(), "episode");
        assert_eq!(MediaKind::Concert.as_str(), "concert");
        assert_eq!(MediaKind::TvShow.to_string(), "tvshow");
    }

    #[test]
    fn nfo_values_override_scanner_values() {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::from("/movies/Alien"),
            PathBuf::from("/movies/Alien/a.mkv"),
            "Alien Guess".to_owned(),
        );
        item.year = Some(1979);

        let mut nfo = MediaItem::new(
            MediaKind::Movie,
            PathBuf::new(),
            PathBuf::new(),
            "Alien".to_owned(),
        );
        nfo.year = Some(1979);
        nfo.overview = Some("A crew encounters a hostile alien.".to_owned());
        nfo.ids.imdb = Some("tt0078748".to_owned());
        nfo.genres = vec!["Horror".to_owned()];
        item.apply_nfo(&nfo);

        assert_eq!(item.title, "Alien");
        assert_eq!(
            item.overview.as_deref(),
            Some("A crew encounters a hostile alien.")
        );
        assert_eq!(item.ids.imdb.as_deref(), Some("tt0078748"));
        assert_eq!(item.genres, vec!["Horror"]);
    }

    #[test]
    fn empty_nfo_fields_leave_scanner_values_untouched() {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::new(),
            PathBuf::new(),
            "Scanned Title".to_owned(),
        );
        item.genres = vec!["Action".to_owned()];
        let nfo = MediaItem::new(
            MediaKind::Movie,
            PathBuf::new(),
            PathBuf::new(),
            String::new(),
        );
        item.apply_nfo(&nfo);
        assert_eq!(item.title, "Scanned Title");
        assert_eq!(item.genres, vec!["Action"]);
    }

    #[test]
    fn primary_rating_prefers_the_default_entry() {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::new(),
            PathBuf::new(),
            String::new(),
        );
        item.ratings = vec![
            Rating {
                source: "imdb".to_owned(),
                value: 5.8,
                votes: 1641,
                max: 10.0,
                is_default: false,
            },
            Rating {
                source: "themoviedb".to_owned(),
                value: 7.2,
                votes: 3400,
                max: 10.0,
                is_default: true,
            },
        ];
        assert_eq!(item.primary_rating().unwrap().source, "themoviedb");
    }
}
