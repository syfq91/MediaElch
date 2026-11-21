//! Kodi NFO reader. Produces a `MediaItem` draft that is merged over scanner
//! output by the server; NFO values always win over filename guesses.

use std::fs;
use std::path::{Path, PathBuf};

use mediaelch_core::{
    Credit, CreditKind, ExternalIds, ImageKind, ImageRef, MediaItem, MediaKind, Rating,
};
use roxmltree::{Document, Node};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NfoError {
    #[error("cannot read NFO file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid XML in {}: {source}", path.display())]
    Xml {
        path: PathBuf,
        #[source]
        source: roxmltree::Error,
    },
}

pub struct NfoReader;

impl NfoReader {
    pub fn read(path: &Path, kind: MediaKind) -> Result<MediaItem, NfoError> {
        let content = fs::read_to_string(path).map_err(|source| NfoError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let document = Document::parse(&content).map_err(|source| NfoError::Xml {
            path: path.to_path_buf(),
            source,
        })?;

        let detail = detail_root(&document);
        let directory = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut item = MediaItem::new(
            kind,
            directory.clone(),
            path.to_path_buf(),
            child_text(detail, "title").unwrap_or_default(),
        );

        item.sort_title = child_text(detail, "sorttitle");
        item.original_title =
            child_text(detail, "originaltitle").or_else(|| child_text(detail, "showtitle"));
        item.year = child_text(detail, "year").and_then(|value| value.parse().ok());
        item.released = child_text(detail, "premiered").or_else(|| child_text(detail, "aired"));
        if item.year.is_none() {
            item.year = item
                .released
                .as_ref()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok());
        }
        item.overview = child_text(detail, "plot");
        item.runtime_min = child_text(detail, "runtime").and_then(|value| value.parse().ok());
        item.certification = child_text(detail, "mpaa");
        item.season_number = child_text(detail, "season").and_then(|value| value.parse().ok());
        item.episode_number = child_text(detail, "episode").and_then(|value| value.parse().ok());
        item.nfo_path = Some(path.to_path_buf());
        item.nfo_mtime = fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        item.genres = child_texts(detail, "genre");
        item.tags = child_texts(detail, "tag");
        item.studios = child_texts(detail, "studio");
        item.countries = child_texts(detail, "country");

        item.ratings = ratings(detail);
        item.ids = external_ids(detail);
        item.credits = credits(detail, &directory, &mut item.images);
        item.images.extend(images(detail, &directory));

        Ok(item)
    }
}

fn detail_root<'doc, 'input>(document: &'doc Document<'input>) -> Node<'input, 'doc> {
    for tag in [
        "episodedetails",
        "musicvideodetails",
        "movie",
        "tvshow",
        "artist",
        "album",
    ] {
        if let Some(node) = document.descendants().find(|node| node.has_tag_name(tag)) {
            return node;
        }
    }
    document.root_element()
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn child_texts(node: Node<'_, '_>, tag: &str) -> Vec<String> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == tag)
        .filter_map(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect()
}

fn child_node<'input, 'doc>(node: Node<'input, 'doc>, tag: &str) -> Option<Node<'input, 'doc>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
}

fn ratings(detail: Node<'_, '_>) -> Vec<Rating> {
    let mut ratings = Vec::new();
    if let Some(list) = child_node(detail, "ratings") {
        for node in list
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "rating")
        {
            let value = child_text(node, "value")
                .or_else(|| node.text().map(|text| text.trim().to_owned()))
                .and_then(|text| text.parse::<f64>().ok());
            let Some(value) = value else { continue };
            ratings.push(Rating {
                source: node
                    .attribute("name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or("default")
                    .to_owned(),
                value,
                votes: child_text(node, "votes")
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(0),
                max: node
                    .attribute("max")
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(10.0),
                is_default: matches!(node.attribute("default"), Some("true" | "1")),
            });
        }
    }

    if ratings.is_empty() {
        if let Some(value) = child_text(detail, "rating").and_then(|text| text.parse::<f64>().ok())
        {
            ratings.push(Rating {
                source: "default".to_owned(),
                value,
                votes: child_text(detail, "votes")
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(0),
                max: 10.0,
                is_default: true,
            });
        }
    }
    ratings
}

fn external_ids(detail: Node<'_, '_>) -> ExternalIds {
    let mut ids = ExternalIds::default();
    for node in detail
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "uniqueid")
    {
        let value = node.text().map(str::trim).filter(|text| !text.is_empty());
        match node.attribute("type") {
            Some("imdb") => ids.imdb = value.map(str::to_owned),
            Some("tmdb") => ids.tmdb = value.and_then(|text| text.parse().ok()),
            Some("tvmaze") => ids.tvmaze = value.and_then(|text| text.parse().ok()),
            _ => {}
        }
    }
    if ids.imdb.is_none() {
        if let Some(id) = child_text(detail, "id") {
            if id.starts_with("tt") {
                ids.imdb = Some(id);
            }
        }
    }
    ids
}

fn credits(detail: Node<'_, '_>, directory: &Path, images: &mut Vec<ImageRef>) -> Vec<Credit> {
    let mut credits = Vec::new();
    for node in detail
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "actor")
    {
        let Some(name) = child_text(node, "name") else {
            continue;
        };
        let thumb = child_text(node, "thumb");
        if let Some(url) = &thumb {
            let local_path = directory.join(".actors").join(format!("{name}.jpg"));
            images.push(ImageRef {
                kind: ImageKind::Actor,
                season: None,
                source_url: Some(url.clone()),
                local_path,
                language: None,
            });
        }
        credits.push(Credit {
            kind: CreditKind::Actor,
            name: name.clone(),
            role: child_text(node, "role"),
            thumb_url: thumb,
            ordering: child_text(node, "order")
                .and_then(|text| text.parse().ok())
                .unwrap_or(0),
        });
    }
    for (tag, kind) in [
        ("director", CreditKind::Director),
        ("credits", CreditKind::Writer),
    ] {
        for name in child_texts(detail, tag) {
            credits.push(Credit {
                kind,
                name,
                role: None,
                thumb_url: None,
                ordering: 0,
            });
        }
    }
    credits
}

fn images(detail: Node<'_, '_>, directory: &Path) -> Vec<ImageRef> {
    let mut images = Vec::new();
    for node in detail
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "thumb")
    {
        let Some(url) = node.text().map(str::trim).filter(|text| !text.is_empty()) else {
            continue;
        };
        let kind = match node.attribute("aspect") {
            Some("poster") => ImageKind::Poster,
            Some("banner") => ImageKind::Banner,
            _ => ImageKind::Fanart,
        };
        images.push(ImageRef {
            kind,
            season: None,
            source_url: Some(url.to_owned()),
            local_path: local_image_path(directory, kind, None),
            language: node.attribute("lang").map(str::to_owned),
        });
    }
    if let Some(fanart) = child_node(detail, "fanart") {
        for node in fanart
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "thumb")
        {
            let Some(url) = node.text().map(str::trim).filter(|text| !text.is_empty()) else {
                continue;
            };
            images.push(ImageRef {
                kind: ImageKind::Fanart,
                season: None,
                source_url: Some(url.to_owned()),
                local_path: local_image_path(directory, ImageKind::Fanart, None),
                language: node.attribute("lang").map(str::to_owned),
            });
        }
    }
    images
}

fn local_image_path(directory: &Path, kind: ImageKind, season: Option<u32>) -> PathBuf {
    let name = match kind {
        ImageKind::Poster => "poster.jpg",
        ImageKind::Fanart => "fanart.jpg",
        ImageKind::Banner => "banner.jpg",
        ImageKind::SeasonPoster => "season-poster.jpg",
        ImageKind::EpisodeThumb => "thumb.jpg",
        ImageKind::Logo => "logo.png",
        ImageKind::ClearArt => "clearart.png",
        ImageKind::Actor => "actor.jpg",
    };
    let candidate = directory.join(match season {
        Some(season) => format!("season{season:02}-{name}"),
        None => name.to_owned(),
    });
    if candidate.is_file() {
        candidate
    } else {
        directory.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/resources")
            .join(relative)
    }

    #[test]
    fn reads_all_movie_fields() {
        let item =
            NfoReader::read(&resource("movie/kodi_v18_movie_all.nfo"), MediaKind::Movie).unwrap();
        assert_eq!(item.title, "Allegiant");
        assert_eq!(item.original_title.as_deref(), Some("AllegiantOriginal"));
        assert_eq!(item.sort_title.as_deref(), Some("TmovieFc10"));
        assert_eq!(item.year, Some(2016));
        assert_eq!(item.released.as_deref(), Some("2016-03-09"));
        assert_eq!(item.runtime_min, Some(88));
        assert_eq!(item.certification.as_deref(), Some("Rated M"));
        assert_eq!(item.ids.imdb.as_deref(), Some("tt3410834"));
        assert_eq!(item.genres, vec!["Adventure", "Science Fiction"]);
        assert_eq!(item.countries, vec!["United States of America"]);
        assert_eq!(item.tags, vec!["Best Tag"]);
        assert_eq!(item.ratings.len(), 3);
        assert_eq!(item.ratings[0].source, "imdb");
        assert!(item.ratings[0].is_default);
        assert_eq!(item.ratings[0].value, 5.8);
        assert_eq!(item.credits.len(), 6); // 2 actors + 3 writers + 1 director
        assert_eq!(item.credits[0].name, "Shailene Woodley");
        assert_eq!(
            item.credits[0].role.as_deref(),
            Some("Beatrice \"Tris\" Prior")
        );
    }

    #[test]
    fn reads_show_ratings_and_ids() {
        let item =
            NfoReader::read(&resource("show/kodi_v18_show_all.nfo"), MediaKind::TvShow).unwrap();
        assert_eq!(item.title, "Angel");
        assert_eq!(item.ids.tmdb, Some(2426));
        assert_eq!(item.ids.tvmaze, Some(428));
        assert_eq!(item.ids.imdb.as_deref(), Some("tt0162065"));
        assert_eq!(item.ratings.len(), 1);
        assert_eq!(item.ratings[0].value, 8.6);
        assert_eq!(item.genres, vec!["Action", "Comedy", "Drama"]);
    }

    #[test]
    fn reads_episode_details_from_episodedetails_wrapper() {
        let item = NfoReader::read(
            &resource("show/kodi_v18_episode_American_Dad_S02E01.nfo"),
            MediaKind::Episode,
        )
        .unwrap();
        assert_eq!(item.title, "Camp Refoogee");
        assert_eq!(item.season_number, Some(2));
        assert_eq!(item.episode_number, Some(1));
        assert_eq!(item.ids.tmdb, Some(66034));
        assert_eq!(item.certification.as_deref(), Some("TV-14"));
        assert_eq!(item.released.as_deref(), Some("2005-09-11"));
    }

    #[test]
    fn reads_v20_show_schema_as_well() {
        let item =
            NfoReader::read(&resource("show/kodi_v20_show_all.nfo"), MediaKind::TvShow).unwrap();
        assert_eq!(item.title, "Angel");
        assert_eq!(item.year, Some(1999));
    }

    #[test]
    fn empty_nfo_produces_an_empty_draft() {
        let item = NfoReader::read(
            &resource("movie/kodi_v18_movie_empty.nfo"),
            MediaKind::Movie,
        )
        .unwrap();
        assert!(item.title.is_empty());
        assert!(item.ratings.is_empty());
        assert_eq!(item.kind, MediaKind::Movie);
    }
}
