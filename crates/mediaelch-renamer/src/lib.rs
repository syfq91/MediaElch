//! MediaElch placeholder renamer with dry-run and rollback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mediaelch_core::MediaItem;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenameError {
    #[error("invalid pattern: {0}")]
    InvalidPattern(String),
    #[error("unknown placeholder: {0}")]
    UnknownPlaceholder(String),
    #[error("cannot rename {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    Number(f64),
    Empty,
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Number(number) => *number != 0.0,
            Self::Empty => false,
        }
    }

    fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::List(items) => items.join(", "),
            Self::Number(number) => {
                if number.fract() == 0.0 {
                    format!("{}", *number as i64)
                } else {
                    format!("{number:.2}")
                }
            }
            Self::Empty => String::new(),
        }
    }
}

#[derive(Debug, Clone)]
enum Node {
    Text(String),
    Value(String),
    Conditional { name: String, body: Vec<Node> },
}

fn parse_nodes(
    chars: &[char],
    position: &mut usize,
    nested: bool,
) -> Result<(Vec<Node>, Option<String>), RenameError> {
    let mut nodes = Vec::new();
    let mut text = String::new();

    while *position < chars.len() {
        match chars[*position] {
            '}' => return Err(RenameError::InvalidPattern("unexpected '}'".to_owned())),
            '{' => {
                flush_text(&mut nodes, &mut text);
                *position += 1;
                if *position < chars.len() && chars[*position] == '/' {
                    if !nested {
                        return Err(RenameError::InvalidPattern("unexpected '{/'".to_owned()));
                    }
                    *position += 1;
                    let start = *position;
                    while *position < chars.len() && chars[*position] != '}' {
                        *position += 1;
                    }
                    if *position >= chars.len() {
                        return Err(RenameError::InvalidPattern("missing '}'".to_owned()));
                    }
                    let closing_name: String = chars[start..*position].iter().collect();
                    *position += 1;
                    return Ok((nodes, Some(closing_name)));
                }
                let start = *position;
                while *position < chars.len() && chars[*position] != '}' {
                    *position += 1;
                }
                if *position >= chars.len() {
                    return Err(RenameError::InvalidPattern("missing '}'".to_owned()));
                }
                let name: String = chars[start..*position].iter().collect();
                *position += 1;
                if name.trim().is_empty() {
                    return Err(RenameError::InvalidPattern("empty condition".to_owned()));
                }
                let (body, closing) = parse_nodes(chars, position, true)?;
                if closing.as_deref() != Some(name.trim()) {
                    return Err(RenameError::InvalidPattern(format!(
                        "expected '{{/{}}}', found '{{/{}}}'",
                        name.trim(),
                        closing.unwrap_or_default()
                    )));
                }
                nodes.push(Node::Conditional {
                    name: name.trim().to_owned(),
                    body,
                });
            }
            '<' => {
                flush_text(&mut nodes, &mut text);
                *position += 1;
                let start = *position;
                while *position < chars.len() && chars[*position] != '>' {
                    *position += 1;
                }
                if *position >= chars.len() {
                    return Err(RenameError::InvalidPattern("missing '>'".to_owned()));
                }
                let name: String = chars[start..*position].iter().collect();
                *position += 1;
                if name.trim().is_empty() {
                    return Err(RenameError::InvalidPattern("empty placeholder".to_owned()));
                }
                nodes.push(Node::Value(name.trim().to_owned()));
            }
            '>' => return Err(RenameError::InvalidPattern("unexpected '>'".to_owned())),
            c => {
                text.push(c);
                *position += 1;
            }
        }
    }
    flush_text(&mut nodes, &mut text);
    if nested {
        return Err(RenameError::InvalidPattern(
            "missing closing condition".to_owned(),
        ));
    }
    Ok((nodes, None))
}

fn flush_text(nodes: &mut Vec<Node>, text: &mut String) {
    if !text.is_empty() {
        nodes.push(Node::Text(std::mem::take(text)));
    }
}

/// Evaluate a pattern against a set of placeholder values.
pub fn evaluate(pattern: &str, values: &HashMap<String, Value>) -> Result<String, RenameError> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut position = 0;
    let (nodes, _) = parse_nodes(&chars, &mut position, false)?;
    let mut out = String::new();
    render(&nodes, values, &mut out)?;
    Ok(out)
}

fn render(
    nodes: &[Node],
    values: &HashMap<String, Value>,
    out: &mut String,
) -> Result<(), RenameError> {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Value(expression) => out.push_str(&resolve(expression, values)?),
            Node::Conditional { name, body } => {
                let value = values
                    .get(name)
                    .ok_or_else(|| RenameError::UnknownPlaceholder(name.clone()))?;
                if value.truthy() {
                    render(body, values, out)?;
                }
            }
        }
    }
    Ok(())
}

fn resolve(expression: &str, values: &HashMap<String, Value>) -> Result<String, RenameError> {
    let mut parts = expression.split('|');
    let head = parts.next().unwrap_or_default().trim();
    let functions: Vec<&str> = parts.map(str::trim).collect();

    let (name, access) = match head.find(['[', '.', ':']) {
        Some(index) => (&head[..index], &head[index..]),
        None => (head, ""),
    };
    let value = values
        .get(name)
        .ok_or_else(|| RenameError::UnknownPlaceholder(name.to_owned()))?;
    let mut text = apply_access(value, access);

    for function in functions {
        text = match function {
            "lower" => text.to_lowercase(),
            "upper" => text.to_uppercase(),
            other => {
                return Err(RenameError::InvalidPattern(format!(
                    "unknown function: {other}"
                )))
            }
        };
    }
    Ok(text)
}

fn apply_access(value: &Value, access: &str) -> String {
    if let Some(inner) = access
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        let item = match value {
            Value::List(items) => {
                let index = inner
                    .split(':')
                    .next()
                    .and_then(|part| part.parse::<usize>().ok())
                    .unwrap_or(0);
                items.get(index).cloned()
            }
            Value::Text(text) => {
                let index = inner.parse::<usize>().ok().unwrap_or(0);
                text.chars().nth(index).map(|c| c.to_string())
            }
            _ => None,
        };
        return item.unwrap_or_default();
    }
    if let Some(field) = access.strip_prefix('.') {
        return match (value, field) {
            (Value::Text(text), "name") => text.clone(),
            _ => String::new(),
        };
    }
    if let Some(format) = access.strip_prefix(':') {
        let (width, decimals) = match format.split_once(',') {
            Some((width, decimals)) => (
                width.trim_start_matches('0').parse::<usize>().ok(),
                decimals.parse::<usize>().ok(),
            ),
            None => (format.trim_start_matches('0').parse::<usize>().ok(), None),
        };
        let number = match value {
            Value::Number(number) => *number,
            _ => return value.as_text(),
        };
        let mut rendered = match decimals {
            Some(decimals) => format!("{number:.decimals$}"),
            None => format!("{}", number as i64),
        };
        if let Some(width) = width {
            if rendered.len() < width {
                let padding = "0".repeat(width - rendered.len());
                rendered = format!("{padding}{rendered}");
            }
        }
        return rendered;
    }
    value.as_text()
}

/// Build the placeholder map from a media item.
pub fn values_from_item(item: &MediaItem) -> HashMap<String, Value> {
    let mut values = HashMap::new();
    values.insert("title".to_owned(), Value::Text(item.title.clone()));
    values.insert(
        "originaltitle".to_owned(),
        item.original_title
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "sorttitle".to_owned(),
        item.sort_title
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "year".to_owned(),
        item.year
            .map(|y| Value::Number(y as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "premiered".to_owned(),
        item.released
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "runtime".to_owned(),
        item.runtime_min
            .map(|n| Value::Number(n as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "imdbId".to_owned(),
        item.ids
            .imdb
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "tmdbId".to_owned(),
        item.ids
            .tmdb
            .map(|id| Value::Number(id as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "tvmazeId".to_owned(),
        item.ids
            .tvmaze
            .map(|id| Value::Number(id as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "certification".to_owned(),
        item.certification
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "season".to_owned(),
        item.season_number
            .map(|n| Value::Number(n as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "episode".to_owned(),
        item.episode_number
            .map(|n| Value::Number(n as f64))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "extension".to_owned(),
        item.primary_file
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| Value::Text(ext.to_owned()))
            .unwrap_or(Value::Empty),
    );
    values.insert(
        "rating".to_owned(),
        item.primary_rating()
            .map(|rating| Value::Number(rating.value))
            .unwrap_or(Value::Empty),
    );
    values.insert("genres".to_owned(), Value::List(item.genres.clone()));
    values.insert("tags".to_owned(), Value::List(item.tags.clone()));
    values.insert("studios".to_owned(), Value::List(item.studios.clone()));
    values.insert(
        "actors".to_owned(),
        Value::List(
            item.credits
                .iter()
                .filter(|credit| credit.kind == mediaelch_core::CreditKind::Actor)
                .map(|credit| credit.name.clone())
                .collect(),
        ),
    );
    values
}

/// Replace characters that are unsafe in filenames on common filesystems.
pub fn sanitize(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.chars().count() > 200 {
        trimmed.chars().take(200).collect()
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameEntry {
    pub item_id: i64,
    pub from: PathBuf,
    pub to: PathBuf,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplySummary {
    pub planned: usize,
    pub renamed: usize,
    pub failed: Vec<String>,
}

/// Compute a rename for one item. `file_pattern` must include the extension,
/// e.g. `<title> (<year>).<extension>`.
pub fn plan_rename(item: &MediaItem, file_pattern: &str) -> Result<RenameEntry, RenameError> {
    let values = values_from_item(item);
    let rendered = evaluate(file_pattern, &values)?;
    let name = sanitize(&rendered);
    let from = item.primary_file.clone();
    let to = from.parent().unwrap_or_else(|| Path::new(".")).join(name);
    Ok(RenameEntry {
        item_id: item.id,
        from,
        to,
        error: None,
    })
}

/// Apply a rename plan. On failure, already-renamed files are moved back.
pub fn apply(entries: &[RenameEntry], dry_run: bool) -> Result<ApplySummary, RenameError> {
    let mut summary = ApplySummary {
        planned: entries.len(),
        renamed: 0,
        failed: Vec::new(),
    };
    if dry_run {
        return Ok(summary);
    }

    let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in entries {
        if entry.from == entry.to {
            continue;
        }
        match std::fs::rename(&entry.from, &entry.to) {
            Ok(()) => {
                completed.push((entry.to.clone(), entry.from.clone()));
                summary.renamed += 1;
            }
            Err(error) => {
                summary
                    .failed
                    .push(format!("{}: {error}", entry.from.display()));
                for (new_path, old_path) in completed.iter().rev() {
                    let _ = std::fs::rename(new_path, old_path);
                }
                return Ok(summary);
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaelch_core::{Credit, CreditKind, ExternalIds, MediaKind};
    use std::path::PathBuf;

    fn movie() -> MediaItem {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::from("/movies/Alien (1979)"),
            PathBuf::from("/movies/Alien (1979)/original.mkv"),
            "Alien".to_owned(),
        );
        item.year = Some(1979);
        item.ids = ExternalIds {
            imdb: Some("tt0078748".to_owned()),
            tmdb: None,
            tvmaze: None,
        };
        item.genres = vec!["Horror".to_owned(), "Sci-Fi".to_owned()];
        item.credits.push(Credit {
            kind: CreditKind::Actor,
            name: "Sigourney Weaver".to_owned(),
            role: None,
            thumb_url: None,
            ordering: 0,
        });
        item
    }

    #[test]
    fn evaluates_basic_pattern() {
        let values = values_from_item(&movie());
        assert_eq!(
            evaluate("<title> (<year>).<extension>", &values).unwrap(),
            "Alien (1979).mkv"
        );
    }

    #[test]
    fn conditionals_include_and_omit_their_body() {
        let values = values_from_item(&movie());
        assert_eq!(
            evaluate("<title>{imdbId} [imdbId-<imdbId>]{/imdbId}", &values).unwrap(),
            "Alien [imdbId-tt0078748]"
        );
        let mut values = values;
        values.insert("imdbId".to_owned(), Value::Empty);
        assert_eq!(
            evaluate("<title>{imdbId} [imdbId-<imdbId>]{/imdbId}", &values).unwrap(),
            "Alien"
        );
    }

    #[test]
    fn functions_and_list_access_work() {
        let values = values_from_item(&movie());
        assert_eq!(evaluate("<title|lower>", &values).unwrap(), "alien");
        assert_eq!(evaluate("<genres[0]>", &values).unwrap(), "Horror");
        assert_eq!(
            evaluate("<actors[0]>", &values).unwrap(),
            "Sigourney Weaver"
        );
    }

    #[test]
    fn unknown_placeholders_are_hard_errors() {
        let values = values_from_item(&movie());
        assert!(matches!(
            evaluate("<atitle>", &values),
            Err(RenameError::UnknownPlaceholder(_))
        ));
    }

    #[test]
    fn mismatched_conditions_are_rejected() {
        let values = values_from_item(&movie());
        assert!(evaluate("{title}value{/year}", &values).is_err());
        assert!(evaluate("{title}value", &values).is_err());
    }

    #[test]
    fn sanitize_replaces_unsafe_characters() {
        assert_eq!(sanitize("a/b:c*?"), "a_b_c__");
    }

    #[test]
    fn dry_run_does_not_touch_files() {
        let temp = tempfile::tempdir().unwrap();
        let mut item = movie();
        item.primary_file = temp.path().join("original.mkv");
        let entry = plan_rename(&item, "<title>.<extension>").unwrap();
        let summary = apply(std::slice::from_ref(&entry), true).unwrap();
        assert_eq!(summary.planned, 1);
        assert_eq!(summary.renamed, 0);
        assert!(!entry.from.exists());
    }

    #[test]
    fn apply_renames_files() {
        let temp = tempfile::tempdir().unwrap();
        let mut item = movie();
        item.primary_file = temp.path().join("original.mkv");
        std::fs::write(&item.primary_file, b"video").unwrap();
        let entry = plan_rename(&item, "<title>.<extension>").unwrap();
        let summary = apply(&[entry], false).unwrap();
        assert_eq!(summary.renamed, 1);
        assert!(temp.path().join("Alien.mkv").is_file());
    }
}
