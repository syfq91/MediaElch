//! CSV and HTML export of library items.

use std::path::Path;

use mediaelch_core::MediaItem;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("invalid export template: {0}")]
    InvalidTemplate(String),
    #[error("cannot write export file {}: {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Html,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Html => "html",
        }
    }
}

/// Minimal built-in HTML theme used when no custom theme is supplied.
pub const DEFAULT_HTML_TEMPLATE: &str =
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>MediaElch export</title></head><body><h1>MediaElch export</h1><ul>{{#items}}\
<li><strong>{{ title }}</strong> ({{ year }}) — {{ overview }}</li>{{/items}}</ul></body></html>";

/// Build a CSV document with RFC 4180-style quoting.
pub fn to_csv(items: &[MediaItem], separator: char) -> String {
    let columns = [
        "title",
        "original_title",
        "year",
        "premiered",
        "rating",
        "votes",
        "certification",
        "genres",
        "studios",
        "imdb_id",
        "tmdb_id",
        "path",
    ];
    let mut out = String::new();
    out.push_str(&columns.join(&separator.to_string()));
    out.push('\n');

    for item in items {
        let rating = item.primary_rating();
        let fields = [
            item.title.clone(),
            item.original_title.clone().unwrap_or_default(),
            item.year.map(|year| year.to_string()).unwrap_or_default(),
            item.released.clone().unwrap_or_default(),
            rating.map(|r| r.value.to_string()).unwrap_or_default(),
            rating.map(|r| r.votes.to_string()).unwrap_or_default(),
            item.certification.clone().unwrap_or_default(),
            item.genres.join("; "),
            item.studios.join("; "),
            item.ids.imdb.clone().unwrap_or_default(),
            item.ids.tmdb.map(|id| id.to_string()).unwrap_or_default(),
            item.primary_file.to_string_lossy().into_owned(),
        ];
        let quoted: Vec<String> = fields.iter().map(|field| quote(field, separator)).collect();
        out.push_str(&quoted.join(&separator.to_string()));
        out.push('\n');
    }
    out
}

fn quote(field: &str, separator: char) -> String {
    if field.contains(separator)
        || field.contains('"')
        || field.contains('\n')
        || field.contains('\r')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Render a simple template. `{{#items}}…{{/items}}` repeats its body for every
/// item; `{{ key }}` is replaced with an escaped value inside the block.
pub fn to_html(items: &[MediaItem], template: &str) -> Result<String, ExportError> {
    let Some(open) = template.find("{{#items}}") else {
        return Err(ExportError::InvalidTemplate(
            "missing {{#items}}".to_owned(),
        ));
    };
    let after_open = open + "{{#items}}".len();
    let Some(close_relative) = template[after_open..].find("{{/items}}") else {
        return Err(ExportError::InvalidTemplate(
            "missing {{/items}}".to_owned(),
        ));
    };
    let close = after_open + close_relative;
    let head = &template[..open];
    let body = &template[after_open..close];
    let tail = &template[close + "{{/items}}".len()..];

    let mut rendered = String::new();
    rendered.push_str(head);
    for item in items {
        rendered.push_str(&render_item(body, item));
    }
    rendered.push_str(tail);
    Ok(rendered)
}

fn render_item(template: &str, item: &MediaItem) -> String {
    let mut out = template.to_owned();
    let values = [
        ("title", item.title.clone()),
        (
            "original_title",
            item.original_title.clone().unwrap_or_default(),
        ),
        ("year", item.year.map(|y| y.to_string()).unwrap_or_default()),
        ("overview", item.overview.clone().unwrap_or_default()),
        ("genres", item.genres.join(", ")),
        ("imdb_id", item.ids.imdb.clone().unwrap_or_default()),
        ("path", item.primary_file.to_string_lossy().into_owned()),
    ];
    for (key, value) in values {
        let placeholder = format!("{{{{ {key} }}}}");
        out = out.replace(&placeholder, &escape_html(&value));
    }
    out
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn write_file(path: &Path, content: &str) -> Result<(), ExportError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ExportError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, content).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaelch_core::{MediaKind, Rating};
    use std::path::PathBuf;

    fn movie(title: &str) -> MediaItem {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::from("/movies"),
            PathBuf::from("/movies/a.mkv"),
            title.to_owned(),
        );
        item.year = Some(1979);
        item.genres = vec!["Horror".to_owned(), "Sci-Fi".to_owned()];
        item.overview = Some("A & B <C>".to_owned());
        item.ratings = vec![Rating {
            source: "imdb".to_owned(),
            value: 8.5,
            votes: 1000,
            max: 10.0,
            is_default: true,
        }];
        item
    }

    #[test]
    fn csv_quotes_commas_and_quotes() {
        let mut item = movie("Alien, The");
        item.certification = Some("Say \"hi\"".to_owned());
        let csv = to_csv(std::slice::from_ref(&item), ',');
        assert!(csv.contains("\"Alien, The\""));
        assert!(csv.contains("\"Say \"\"hi\"\"\""));
        assert!(csv.lines().count() == 2);
    }

    #[test]
    fn csv_contains_a_header_and_all_columns() {
        let csv = to_csv(&[movie("Alien")], ';');
        assert!(csv.starts_with("title;original_title;year;"));
        assert!(csv.contains("Horror; Sci-Fi"));
    }

    #[test]
    fn html_repeats_the_item_block_and_escapes_values() {
        let template =
            "<ul>{{#items}}<li>{{ title }} ({{ year }}) — {{ overview }}</li>{{/items}}</ul>";
        let html = to_html(&[movie("Alien"), movie("Toy Story")], template).unwrap();
        assert_eq!(html.matches("<li>").count(), 2);
        assert!(html.contains("Alien (1979)"));
        assert!(html.contains("A &amp; B &lt;C&gt;"));
    }

    #[test]
    fn invalid_templates_are_rejected() {
        assert!(to_html(&[], "<p>{{ title }}</p>").is_err());
    }

    #[test]
    fn writes_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("export").join("movies.csv");
        write_file(&path, "header\n").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "header\n");
    }
}
