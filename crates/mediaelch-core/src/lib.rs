//! Domain model for the headless MediaElch server.
//!
//! Pure data types only: no I/O, no framework dependencies, no Qt.

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
}
