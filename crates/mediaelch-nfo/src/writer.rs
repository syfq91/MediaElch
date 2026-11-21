//! Deterministic Kodi NFO writer.

use std::fs;
use std::path::Path;

use mediaelch_core::{ImageKind, MediaItem, MediaKind};

use crate::NfoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KodiVersion {
    V18,
    V20,
    V21,
}

impl KodiVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V18 => "18",
            Self::V20 => "20",
            Self::V21 => "21",
        }
    }
}

pub struct NfoWriter;

impl NfoWriter {
    /// Render an item as a Kodi NFO document. Output is deterministic: tag
    /// order is fixed and no timestamps are emitted.
    pub fn write(item: &MediaItem, version: KodiVersion) -> String {
        let mut out =
            String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        let root = match item.kind {
            MediaKind::Movie | MediaKind::Concert => "movie",
            MediaKind::TvShow => "tvshow",
            MediaKind::Episode => "episodedetails",
        };
        out.push('<');
        out.push_str(root);
        out.push_str(">\n");

        push_text(&mut out, 1, "title", Some(&item.title));
        push_text(&mut out, 1, "originaltitle", item.original_title.as_deref());
        push_text(&mut out, 1, "sorttitle", item.sort_title.as_deref());

        if !item.ratings.is_empty() {
            out.push_str("  <ratings>\n");
            for rating in &item.ratings {
                out.push_str(&format!(
                    "    <rating name=\"{}\" default=\"{}\" max=\"{}\">\n",
                    escape(&rating.source),
                    bool_attr(rating.is_default),
                    rating.max
                ));
                push_text(&mut out, 3, "value", Some(&rating.value.to_string()));
                push_text(&mut out, 3, "votes", Some(&rating.votes.to_string()));
                out.push_str("    </rating>\n");
            }
            out.push_str("  </ratings>\n");
        }

        push_text(
            &mut out,
            1,
            "year",
            item.year.map(|year| year.to_string()).as_deref(),
        );
        push_text(&mut out, 1, "premiered", item.released.as_deref());
        push_text(&mut out, 1, "plot", item.overview.as_deref());
        push_text(
            &mut out,
            1,
            "runtime",
            item.runtime_min.map(|n| n.to_string()).as_deref(),
        );
        push_text(&mut out, 1, "mpaa", item.certification.as_deref());
        push_text(
            &mut out,
            1,
            "season",
            item.season_number.map(|n| n.to_string()).as_deref(),
        );
        push_text(
            &mut out,
            1,
            "episode",
            item.episode_number.map(|n| n.to_string()).as_deref(),
        );

        if let Some(id) = &item.ids.imdb {
            push_attr_text(&mut out, 1, "uniqueid", "type", "imdb", Some(id));
        }
        if let Some(id) = item.ids.tmdb {
            push_attr_text(
                &mut out,
                1,
                "uniqueid",
                "type",
                "tmdb",
                Some(&id.to_string()),
            );
        }
        if let Some(id) = item.ids.tvmaze {
            push_attr_text(
                &mut out,
                1,
                "uniqueid",
                "type",
                "tvmaze",
                Some(&id.to_string()),
            );
        }

        for (tag, values) in [
            ("genre", &item.genres),
            ("country", &item.countries),
            ("studio", &item.studios),
            ("tag", &item.tags),
        ] {
            for value in values {
                push_text(&mut out, 1, tag, Some(value));
            }
        }

        for credit in &item.credits {
            out.push_str("  <actor>\n");
            push_text(&mut out, 2, "name", Some(&credit.name));
            push_text(&mut out, 2, "role", credit.role.as_deref());
            push_text(&mut out, 2, "order", Some(&credit.ordering.to_string()));
            push_text(&mut out, 2, "thumb", credit.thumb_url.as_deref());
            out.push_str("  </actor>\n");
        }

        for image in &item.images {
            match image.kind {
                ImageKind::Fanart => continue,
                kind => {
                    let aspect = match kind {
                        ImageKind::Poster => "poster",
                        ImageKind::Banner => "banner",
                        _ => kind.as_str(),
                    };
                    let url = image.source_url.as_deref().unwrap_or_default();
                    if !url.is_empty() {
                        out.push_str(&format!(
                            "  <thumb aspect=\"{}\">{}</thumb>\n",
                            escape(aspect),
                            escape(url)
                        ));
                    }
                }
            }
        }
        if item
            .images
            .iter()
            .any(|image| image.kind == ImageKind::Fanart)
        {
            out.push_str("  <fanart>\n");
            for image in item
                .images
                .iter()
                .filter(|image| image.kind == ImageKind::Fanart)
            {
                if let Some(url) = &image.source_url {
                    push_text(&mut out, 2, "thumb", Some(url));
                }
            }
            out.push_str("  </fanart>\n");
        }

        out.push_str(&format!(
            "  <generator>MediaElch/{} (Kodi {})</generator>\n",
            env!("CARGO_PKG_VERSION"),
            version.as_str()
        ));
        let element = match item.kind {
            MediaKind::Movie | MediaKind::Concert => "movie",
            MediaKind::TvShow => "tvshow",
            MediaKind::Episode => "episodedetails",
        };
        out.push_str("</");
        out.push_str(element);
        out.push_str(">\n");
        out
    }

    /// Write the NFO to disk atomically, creating parent directories.
    pub fn save(item: &MediaItem, path: &Path, version: KodiVersion) -> Result<(), NfoError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| NfoError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let temporary = path.with_extension("nfo.tmp");
        fs::write(&temporary, Self::write(item, version)).map_err(|source| NfoError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, path).map_err(|source| NfoError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }
}

fn push_text(out: &mut String, indent: usize, tag: &str, value: Option<&str>) {
    if let Some(value) = value {
        if !value.is_empty() {
            out.push_str(&"  ".repeat(indent));
            out.push('<');
            out.push_str(tag);
            out.push('>');
            out.push_str(&escape(value));
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
    }
}

fn push_attr_text(
    out: &mut String,
    indent: usize,
    tag: &str,
    attr: &str,
    value: &str,
    text: Option<&str>,
) {
    if let Some(text) = text {
        out.push_str(&"  ".repeat(indent));
        out.push('<');
        out.push_str(tag);
        out.push(' ');
        out.push_str(attr);
        out.push_str("=\"");
        out.push_str(&escape(value));
        out.push_str("\">");
        out.push_str(&escape(text));
        out.push_str("</");
        out.push_str(tag);
        out.push_str(">\n");
    }
}

fn bool_attr(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NfoReader;
    use mediaelch_core::{ExternalIds, Rating};
    use roxmltree::Document;
    use std::path::PathBuf;

    fn sample_movie() -> MediaItem {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::from("/movies/Alien"),
            PathBuf::from("/movies/Alien/a.mkv"),
            "Alien".to_owned(),
        );
        item.year = Some(1979);
        item.released = Some("1979-05-25".to_owned());
        item.overview = Some("A crew & their ship".to_owned());
        item.certification = Some("Rated R".to_owned());
        item.runtime_min = Some(117);
        item.ids = ExternalIds {
            imdb: Some("tt0078748".to_owned()),
            tmdb: Some(348),
            tvmaze: None,
        };
        item.ratings = vec![Rating {
            source: "imdb".to_owned(),
            value: 8.5,
            votes: 7653,
            max: 10.0,
            is_default: true,
        }];
        item.genres = vec!["Horror".to_owned(), "Sci-Fi".to_owned()];
        item
    }

    #[test]
    fn writes_valid_xml_with_escaped_text() {
        let xml = NfoWriter::write(&sample_movie(), KodiVersion::V18);
        Document::parse(&xml).unwrap();
        assert!(xml.contains("A crew &amp; their ship"));
        assert!(xml.contains("<uniqueid type=\"imdb\">tt0078748</uniqueid>"));
        assert!(xml.contains("<rating name=\"imdb\" default=\"true\" max=\"10\">"));
    }

    #[test]
    fn read_write_read_preserves_core_fields() {
        let original = sample_movie();
        let xml = NfoWriter::write(&original, KodiVersion::V18);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("alien.nfo");
        fs::write(&path, xml).unwrap();

        let parsed = NfoReader::read(&path, MediaKind::Movie).unwrap();
        assert_eq!(parsed.title, original.title);
        assert_eq!(parsed.year, original.year);
        assert_eq!(parsed.released, original.released);
        assert_eq!(parsed.overview, original.overview);
        assert_eq!(parsed.certification, original.certification);
        assert_eq!(parsed.runtime_min, original.runtime_min);
        assert_eq!(parsed.ids, original.ids);
        assert_eq!(parsed.genres, original.genres);
        assert_eq!(parsed.ratings, original.ratings);
    }

    #[test]
    fn writer_output_is_deterministic() {
        let item = sample_movie();
        assert_eq!(
            NfoWriter::write(&item, KodiVersion::V18),
            NfoWriter::write(&item, KodiVersion::V18)
        );
    }

    #[test]
    fn save_creates_directories_and_replaces_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sub").join("alien.nfo");
        NfoWriter::save(&sample_movie(), &path, KodiVersion::V18).unwrap();
        NfoWriter::save(&sample_movie(), &path, KodiVersion::V20).unwrap();
        assert!(path.is_file());
        assert!(!temp.path().join("sub").join("alien.nfo.tmp").exists());
    }

    #[test]
    fn writes_episode_specific_tags() {
        let mut episode = MediaItem::new(
            MediaKind::Episode,
            PathBuf::from("/shows/Oz"),
            PathBuf::from("/shows/Oz/e01.mkv"),
            "Camp Refoogee".to_owned(),
        );
        episode.season_number = Some(2);
        episode.episode_number = Some(1);
        let xml = NfoWriter::write(&episode, KodiVersion::V18);
        assert!(xml.contains("<episodedetails>"));
        assert!(xml.contains("<season>2</season>"));
        assert!(xml.contains("<episode>1</episode>"));
    }
}
