//! Directory scanning and media classification.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use mediaelch_core::{MediaFileRef, MediaItem, MediaKind};
use walkdir::WalkDir;

use crate::filenames::{
    episode_numbers, format_name, season_number, stacked_base_name, without_extension,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ScanRoot {
    pub path: PathBuf,
    /// `auto`, `movie`, `tvshow` or `concert`.
    pub kind: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanConfig {
    pub skip_directories: Vec<String>,
    pub video_extensions: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            skip_directories: [
                ".actors",
                "extras",
                "extrafanart",
                "extrathumbs",
                "featurettes",
                "subs",
                "subtitles",
                "sample",
                "@eadir",
                "#recycle",
                "lost+found",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
            video_extensions: [
                "mkv", "mp4", "avi", "mpeg", "mpg", "m2ts", "ts", "mov", "wmv", "flv", "webm",
                "m4v", "ogm", "strm", "3gp",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub items: Vec<MediaItem>,
    pub directories_scanned: usize,
    pub files_seen: usize,
}

pub struct Scanner {
    config: ScanConfig,
}

impl Scanner {
    pub fn new(config: ScanConfig) -> Self {
        Self { config }
    }

    pub fn scan(&self, roots: &[ScanRoot]) -> ScanResult {
        let mut result = ScanResult::default();
        let mut consumed: HashSet<PathBuf> = HashSet::new();
        let mut shows: HashMap<PathBuf, usize> = HashMap::new();

        for root in roots {
            if !root.path.is_dir() {
                tracing::warn!(path = %root.path.display(), "scan root is not a directory");
                continue;
            }
            let max_depth = if root.recursive { 12 } else { 1 };
            let walker = WalkDir::new(&root.path)
                .follow_links(false)
                .max_depth(max_depth)
                .sort_by_file_name();

            let mut directories: Vec<PathBuf> = walker
                .into_iter()
                .filter_map(|entry| match entry {
                    Ok(entry) if entry.file_type().is_dir() => Some(entry.into_path()),
                    Ok(_) => None,
                    Err(error) => {
                        tracing::warn!(%error, "skipping unreadable scan entry");
                        None
                    }
                })
                .collect();
            directories.sort_by_key(|path| path.components().count());

            for directory in directories {
                if consumed.contains(&directory) || self.is_skipped(&directory) {
                    continue;
                }
                result.directories_scanned += 1;

                let files = self.video_files(&directory);
                result.files_seen += files.len();
                let disc = self.is_disc_directory(&directory);
                if files.is_empty() && !disc {
                    continue;
                }

                let forced_kind = root.kind.as_str();
                let looks_like_season_dir = is_season_directory(&directory);
                let has_episode_names = files.iter().any(|path| {
                    let name = path.to_string_lossy();
                    season_number(&name).is_some() && !episode_numbers(&name).is_empty()
                });

                if forced_kind == "tvshow"
                    || (forced_kind == "auto" && (has_episode_names || looks_like_season_dir))
                {
                    let show_dir = if looks_like_season_dir {
                        directory.parent().unwrap_or(&directory).to_path_buf()
                    } else {
                        directory.clone()
                    };

                    if !shows.contains_key(&show_dir) {
                        let mut show = MediaItem::new(
                            MediaKind::TvShow,
                            show_dir.clone(),
                            show_dir.join("tvshow.nfo"),
                            format_name(&file_name(&show_dir)),
                        );
                        self.attach_nfo(&mut show, &show_dir.join("tvshow.nfo"));
                        shows.insert(show_dir.clone(), result.items.len());
                        consumed.insert(show_dir.clone());
                        result.items.push(show);
                    }

                    // Consume season directories so they are not re-scanned as
                    // movie folders on a later walk iteration.
                    for child in fs::read_dir(&directory).into_iter().flatten().flatten() {
                        if child.path().is_dir() && is_season_directory(&child.path()) {
                            consumed.insert(child.path());
                        }
                    }

                    for file in files {
                        let name = file.to_string_lossy();
                        let episode = episode_numbers(&name).into_iter().next();
                        let mut item = MediaItem::new(
                            MediaKind::Episode,
                            show_dir.clone(),
                            file.clone(),
                            format_name(without_extension(&file_name(&file))),
                        );
                        item.season_number = season_number(&name);
                        item.episode_number = episode;
                        item.files.push(MediaFileRef {
                            path: file.clone(),
                            size: file_size(&file),
                            mtime: file_mtime(&file),
                            stack_part: None,
                            is_extra: false,
                        });
                        let nfo = file.with_extension("nfo");
                        self.attach_nfo(&mut item, &nfo);
                        result.items.push(item);
                    }
                } else {
                    let kind = if forced_kind == "concert" {
                        MediaKind::Concert
                    } else {
                        MediaKind::Movie
                    };

                    if disc {
                        for child in fs::read_dir(&directory).into_iter().flatten().flatten() {
                            let name = child.file_name().to_string_lossy().to_lowercase();
                            if child.path().is_dir()
                                && ["bdmv", "video_ts", "certs", "backup"].contains(&name.as_str())
                            {
                                consumed.insert(child.path());
                            }
                        }
                        let mut item = MediaItem::new(
                            kind,
                            directory.clone(),
                            directory.clone(),
                            format_name(&file_name(&directory)),
                        );
                        self.attach_nfo(&mut item, &directory.join("movie.nfo"));
                        result.items.push(item);
                        continue;
                    }

                    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
                    for file in files {
                        groups
                            .entry(stacked_base_name(&file_name(&file)))
                            .or_default()
                            .push(file);
                    }
                    for (base, mut paths) in groups {
                        paths.sort();
                        let primary = paths[0].clone();
                        let mut item = MediaItem::new(
                            kind,
                            directory.clone(),
                            primary.clone(),
                            title_from_base(&base, &primary),
                        );
                        for (index, path) in paths.into_iter().enumerate() {
                            item.files.push(MediaFileRef {
                                path: path.clone(),
                                size: file_size(&path),
                                mtime: file_mtime(&path),
                                stack_part: Some(index as u32 + 1),
                                is_extra: false,
                            });
                        }
                        let nfo = primary.with_extension("nfo");
                        self.attach_nfo(&mut item, &nfo);
                        result.items.push(item);
                    }
                }
            }
        }
        result
    }

    fn is_skipped(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let lower = name.to_lowercase();
                self.config.skip_directories.contains(&lower)
            })
    }

    fn is_disc_directory(&self, directory: &Path) -> bool {
        let Ok(entries) = fs::read_dir(directory) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                return fs::read_dir(path)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|child| is_disc_marker(&child.file_name().to_string_lossy()));
            }
            is_disc_marker(&entry.file_name().to_string_lossy())
        })
    }

    fn video_files(&self, directory: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        self.config
                            .video_extensions
                            .iter()
                            .any(|known| known.eq_ignore_ascii_case(ext))
                    })
            })
            .collect();
        files.sort();
        files
    }

    fn attach_nfo(&self, item: &mut MediaItem, path: &Path) {
        if path.is_file() {
            item.nfo_path = Some(path.to_path_buf());
            item.nfo_mtime = file_mtime(path);
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
}

fn file_mtime(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

fn is_disc_marker(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "index.bdmv" || lower == "video_ts.ifo"
}

fn is_season_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    lower == "specials"
        || lower.starts_with("season")
        || (lower.starts_with('s')
            && lower[1..].chars().all(|c| c.is_ascii_digit())
            && lower.len() > 1)
}

fn title_from_base(base: &str, original: &Path) -> String {
    let extension = original
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let stem = if base
        .rsplit('.')
        .next()
        .is_some_and(|last| last.eq_ignore_ascii_case(extension))
    {
        without_extension(base)
    } else {
        base
    };
    format_name(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_library(root: &Path) {
        let movie = root.join("Movies").join("Alien (1979)");
        fs::create_dir_all(&movie).unwrap();
        fs::write(movie.join("Alien (1979).mkv"), b"video").unwrap();

        let stacked = root.join("Movies").join("Caprica");
        fs::create_dir_all(&stacked).unwrap();
        fs::write(stacked.join("caprica-cd1.avi"), b"a").unwrap();
        fs::write(stacked.join("caprica-cd2.avi"), b"b").unwrap();
        fs::create_dir_all(stacked.join("extras")).unwrap();
        fs::write(stacked.join("extras").join("trailer.mkv"), b"t").unwrap();

        let show = root.join("Shows").join("Oz");
        fs::create_dir_all(show.join("Season 01")).unwrap();
        fs::write(
            show.join("Season 01").join("Oz.S01E01.Emerald City.mkv"),
            b"e1",
        )
        .unwrap();
        fs::write(show.join("Season 01").join("Oz.S01E02.mkv"), b"e2").unwrap();
        fs::write(show.join("tvshow.nfo"), b"<tvshow></tvshow>").unwrap();
    }

    #[test]
    fn scanner_classifies_movies_stacks_and_episodes() {
        let temp = tempfile::tempdir().unwrap();
        make_library(temp.path());
        let scanner = Scanner::new(ScanConfig::default());
        let result = scanner.scan(&[
            ScanRoot {
                path: temp.path().join("Movies"),
                kind: "movie".to_owned(),
                recursive: true,
            },
            ScanRoot {
                path: temp.path().join("Shows"),
                kind: "tvshow".to_owned(),
                recursive: true,
            },
        ]);

        let movies: Vec<_> = result
            .items
            .iter()
            .filter(|i| i.kind == MediaKind::Movie)
            .collect();
        assert_eq!(movies.len(), 2);
        let caprica = movies.iter().find(|i| i.title == "caprica").unwrap();
        assert_eq!(caprica.files.len(), 2);
        assert_eq!(caprica.files[0].stack_part, Some(1));

        let shows: Vec<_> = result
            .items
            .iter()
            .filter(|i| i.kind == MediaKind::TvShow)
            .collect();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].title, "Oz");
        assert!(shows[0].nfo_path.is_some());

        let episodes: Vec<_> = result
            .items
            .iter()
            .filter(|i| i.kind == MediaKind::Episode)
            .collect();
        assert_eq!(episodes.len(), 2);
        assert!(episodes.iter().all(|e| e.season_number == Some(1)));
        assert!(episodes.iter().all(|e| e.directory == shows[0].directory));
        assert!(episodes.iter().any(|e| e.episode_number == Some(1)));
        assert!(episodes.iter().any(|e| e.episode_number == Some(2)));
    }

    #[test]
    fn scanner_skips_extras_and_detects_disc_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let disc = temp.path().join("Concerts").join("Live");
        fs::create_dir_all(disc.join("BDMV")).unwrap();
        fs::write(disc.join("BDMV").join("index.bdmv"), b"disc").unwrap();
        let scanner = Scanner::new(ScanConfig::default());
        let result = scanner.scan(&[ScanRoot {
            path: temp.path().join("Concerts"),
            kind: "concert".to_owned(),
            recursive: true,
        }]);

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].kind, MediaKind::Concert);
    }
}
