//! Typed read/write access to the media library tables.

use std::path::{Path, PathBuf};

use mediaelch_core::{
    Credit, CreditKind, ExternalIds, ImageKind, ImageRef, MediaFileRef, MediaItem, MediaKind,
    Rating,
};
use rusqlite::{named_params, params, Connection, OptionalExtension, Row};

use crate::DbError;

/// A configured scan root, as stored in `media_directory`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanRoot {
    pub id: i64,
    pub path: PathBuf,
    pub kind: String,
    pub recursive: bool,
    pub last_scan: Option<String>,
}

pub struct MediaRepository<'a> {
    conn: &'a mut Connection,
}

impl<'a> MediaRepository<'a> {
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    /// Insert or update an item plus all of its child rows. The operation is
    /// atomic: a failed child insert rolls back the whole item.
    pub fn upsert(&mut self, item: &MediaItem) -> Result<i64, DbError> {
        let transaction = self.conn.transaction()?;
        let primary = item.primary_rating();

        transaction.execute(
            "INSERT INTO media_item (
                kind, parent_id, directory, primary_file, title, sort_title, original_title,
                year, released, overview, runtime_min, certification, rating, votes, rating_source,
                imdb_id, tmdb_id, tvmaze_id, season_number, episode_number, nfo_path, nfo_mtime, updated_at
             ) VALUES (
                :kind, :parent_id, :directory, :primary_file, :title, :sort_title, :original_title,
                :year, :released, :overview, :runtime_min, :certification, :rating, :votes, :rating_source,
                :imdb_id, :tmdb_id, :tvmaze_id, :season_number, :episode_number, :nfo_path, :nfo_mtime,
                datetime('now')
             )
             ON CONFLICT(primary_file) DO UPDATE SET
                kind=excluded.kind, parent_id=excluded.parent_id, directory=excluded.directory,
                title=excluded.title, sort_title=excluded.sort_title,
                original_title=excluded.original_title, year=excluded.year,
                released=excluded.released, overview=excluded.overview,
                runtime_min=excluded.runtime_min, certification=excluded.certification,
                rating=excluded.rating, votes=excluded.votes, rating_source=excluded.rating_source,
                imdb_id=excluded.imdb_id, tmdb_id=excluded.tmdb_id, tvmaze_id=excluded.tvmaze_id,
                season_number=excluded.season_number, episode_number=excluded.episode_number,
                nfo_path=excluded.nfo_path, nfo_mtime=excluded.nfo_mtime,
                updated_at=datetime('now')",
            named_params! {
                ":kind": item.kind.as_str(),
                ":parent_id": item.parent_id,
                ":directory": path_to_string(&item.directory),
                ":primary_file": path_to_string(&item.primary_file),
                ":title": item.title,
                ":sort_title": item.sort_title,
                ":original_title": item.original_title,
                ":year": item.year,
                ":released": item.released,
                ":overview": item.overview,
                ":runtime_min": item.runtime_min,
                ":certification": item.certification,
                ":rating": primary.map(|r| r.value),
                ":votes": primary.map(|r| r.votes as i64),
                ":rating_source": primary.map(|r| r.source.clone()),
                ":imdb_id": item.ids.imdb,
                ":tmdb_id": item.ids.tmdb.map(|id| id as i64),
                ":tvmaze_id": item.ids.tvmaze.map(|id| id as i64),
                ":season_number": item.season_number.map(|n| n as i64),
                ":episode_number": item.episode_number.map(|n| n as i64),
                ":nfo_path": item.nfo_path.as_ref().map(|p| path_to_string(p)),
                ":nfo_mtime": item.nfo_mtime.map(|t| t as i64),
            },
        )?;

        let id: i64 = transaction.query_row(
            "SELECT id FROM media_item WHERE primary_file = ?1",
            [path_to_string(&item.primary_file)],
            |row| row.get(0),
        )?;

        for table in ["rating", "media_tag", "credit", "image", "media_file"] {
            transaction.execute(&format!("DELETE FROM {table} WHERE item_id = ?1"), [id])?;
        }
        transaction.execute(
            "DELETE FROM stream_detail WHERE file_id NOT IN (SELECT id FROM media_file)",
            [],
        )?;

        for rating in &item.ratings {
            transaction.execute(
                "INSERT INTO rating (item_id, source, value, votes, max_value, is_default)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    rating.source,
                    rating.value,
                    rating.votes as i64,
                    rating.max,
                    i64::from(rating.is_default)
                ],
            )?;
        }
        for (kind, values) in [
            ("genre", &item.genres),
            ("tag", &item.tags),
            ("studio", &item.studios),
            ("country", &item.countries),
        ] {
            for value in values {
                transaction.execute(
                    "INSERT INTO media_tag (item_id, kind, value) VALUES (?1, ?2, ?3)",
                    params![id, kind, value],
                )?;
            }
        }
        for credit in &item.credits {
            transaction.execute(
                "INSERT INTO credit (item_id, kind, name, role, thumb_url, ordering)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    credit.kind.as_str(),
                    credit.name,
                    credit.role,
                    credit.thumb_url,
                    credit.ordering as i64
                ],
            )?;
        }
        for image in &item.images {
            transaction.execute(
                "INSERT INTO image (item_id, kind, season, source_url, local_path, language)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    image.kind.as_str(),
                    image.season.map(|n| n as i64),
                    image.source_url,
                    path_to_string(&image.local_path),
                    image.language
                ],
            )?;
        }
        for file in &item.files {
            transaction.execute(
                "INSERT INTO media_file (item_id, path, size, mtime, stack_part, is_extra)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    path_to_string(&file.path),
                    file.size.map(|n| n as i64),
                    file.mtime.map(|n| n as i64),
                    file.stack_part.map(|n| n as i64),
                    i64::from(file.is_extra)
                ],
            )?;
        }

        transaction.commit()?;
        Ok(id)
    }

    /// Fetch one item with all child rows.
    pub fn get(&self, id: i64) -> Result<Option<MediaItem>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT * FROM media_item WHERE id = ?1")?;
        let mut item = statement
            .query_row([id], |row| self.row_to_item(row))
            .optional()?;
        if let Some(item) = item.as_mut() {
            self.load_children(item)?;
        }
        Ok(item)
    }

    pub fn find_by_primary_file(&self, path: &Path) -> Result<Option<MediaItem>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT * FROM media_item WHERE primary_file = ?1")?;
        statement
            .query_row([path_to_string(path)], |row| self.row_to_item(row))
            .optional()
            .map_err(DbError::from)
    }

    /// Find the database id of an item by kind and directory, used to link
    /// episodes to their show after both have been scanned.
    pub fn find_id(&self, kind: MediaKind, directory: &Path) -> Result<Option<i64>, DbError> {
        self.conn
            .query_row(
                "SELECT id FROM media_item WHERE kind = ?1 AND directory = ?2 LIMIT 1",
                params![kind.as_str(), path_to_string(directory)],
                |row| row.get(0),
            )
            .optional()
            .map_err(DbError::from)
    }

    /// List items without child rows, optionally filtered by kind and a
    /// case-insensitive title/overview search.
    pub fn list(
        &self,
        kind: Option<MediaKind>,
        query: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MediaItem>, DbError> {
        let sql = "SELECT * FROM media_item
                   WHERE (?1 IS NULL OR kind = ?1)
                     AND (?2 IS NULL OR lower(title) LIKE '%' || lower(?2) || '%'
                          OR lower(COALESCE(overview, '')) LIKE '%' || lower(?2) || '%')
                   ORDER BY sort_title, title, id
                   LIMIT ?3 OFFSET ?4";
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map(
            params![kind.map(|k| k.as_str()), query, limit, offset],
            |row| self.row_to_item(row),
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn count(&self, kind: Option<MediaKind>, query: Option<&str>) -> Result<i64, DbError> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM media_item
                 WHERE (?1 IS NULL OR kind = ?1)
                   AND (?2 IS NULL OR lower(title) LIKE '%' || lower(?2) || '%'
                        OR lower(COALESCE(overview, '')) LIKE '%' || lower(?2) || '%')",
                params![kind.map(|k| k.as_str()), query],
                |row| row.get(0),
            )
            .map_err(DbError::from)
    }

    pub fn set_parent(&self, item_id: i64, parent_id: i64) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE media_item SET parent_id = ?2 WHERE id = ?1",
            params![item_id, parent_id],
        )?;
        Ok(())
    }

    /// Remove all items before a full rescan. Child rows are removed by the
    /// foreign-key cascade.
    pub fn clear_items(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM media_item", [])?;
        Ok(())
    }

    pub fn list_roots(&self) -> Result<Vec<ScanRoot>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT id, path, kind, recursive, last_scan FROM media_directory ORDER BY path",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ScanRoot {
                id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                kind: row.get(2)?,
                recursive: row.get::<_, i64>(3)? != 0,
                last_scan: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn upsert_root(&self, path: &Path, kind: &str, recursive: bool) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO media_directory (path, kind, recursive) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET kind=excluded.kind, recursive=excluded.recursive",
            params![path_to_string(path), kind, i64::from(recursive)],
        )?;
        Ok(())
    }

    pub fn touch_root(&self, path: &Path) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE media_directory SET last_scan = datetime('now') WHERE path = ?1",
            [path_to_string(path)],
        )?;
        Ok(())
    }

    fn row_to_item(&self, row: &Row<'_>) -> rusqlite::Result<MediaItem> {
        let kind: String = row.get("kind")?;
        Ok(MediaItem {
            id: row.get("id")?,
            kind: parse_kind(&kind)?,
            parent_id: row.get("parent_id")?,
            directory: PathBuf::from(row.get::<_, String>("directory")?),
            primary_file: PathBuf::from(row.get::<_, String>("primary_file")?),
            title: row.get("title")?,
            sort_title: row.get("sort_title")?,
            original_title: row.get("original_title")?,
            year: row.get("year")?,
            released: row.get("released")?,
            overview: row.get("overview")?,
            runtime_min: row.get("runtime_min")?,
            certification: row.get("certification")?,
            ratings: Vec::new(),
            genres: Vec::new(),
            tags: Vec::new(),
            studios: Vec::new(),
            countries: Vec::new(),
            ids: ExternalIds {
                imdb: row.get("imdb_id")?,
                tmdb: row.get::<_, Option<i64>>("tmdb_id")?.map(|n| n as u64),
                tvmaze: row.get::<_, Option<i64>>("tvmaze_id")?.map(|n| n as u64),
            },
            season_number: row
                .get::<_, Option<i64>>("season_number")?
                .map(|n| n as u32),
            episode_number: row
                .get::<_, Option<i64>>("episode_number")?
                .map(|n| n as u32),
            nfo_path: row.get::<_, Option<String>>("nfo_path")?.map(PathBuf::from),
            nfo_mtime: row.get::<_, Option<i64>>("nfo_mtime")?.map(|n| n as u64),
            credits: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
        })
    }

    fn load_children(&self, item: &mut MediaItem) -> Result<(), DbError> {
        let id = item.id;

        {
            let mut statement = self.conn.prepare(
                "SELECT source, value, votes, max_value, is_default FROM rating WHERE item_id = ?1 ORDER BY id",
            )?;
            let rows = statement.query_map([id], |row| {
                Ok(Rating {
                    source: row.get(0)?,
                    value: row.get(1)?,
                    votes: row.get::<_, i64>(2)? as u64,
                    max: row.get(3)?,
                    is_default: row.get::<_, i64>(4)? != 0,
                })
            })?;
            item.ratings = rows.collect::<Result<Vec<_>, _>>()?;
        }

        {
            let mut statement = self.conn.prepare(
                "SELECT kind, value FROM media_tag WHERE item_id = ?1 ORDER BY kind, value",
            )?;
            let rows = statement.query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (kind, value) = row?;
                match kind.as_str() {
                    "genre" => item.genres.push(value),
                    "tag" => item.tags.push(value),
                    "studio" => item.studios.push(value),
                    "country" => item.countries.push(value),
                    _ => {}
                }
            }
        }

        {
            let mut statement = self.conn.prepare(
                "SELECT kind, name, role, thumb_url, ordering FROM credit WHERE item_id = ?1 ORDER BY ordering, id",
            )?;
            let rows = statement.query_map([id], |row| {
                Ok(Credit {
                    kind: match row.get::<_, String>(0)?.as_str() {
                        "director" => CreditKind::Director,
                        "writer" => CreditKind::Writer,
                        _ => CreditKind::Actor,
                    },
                    name: row.get(1)?,
                    role: row.get(2)?,
                    thumb_url: row.get(3)?,
                    ordering: row.get::<_, i64>(4)? as u32,
                })
            })?;
            item.credits = rows.collect::<Result<Vec<_>, _>>()?;
        }

        {
            let mut statement = self.conn.prepare(
                "SELECT kind, season, source_url, local_path, language FROM image WHERE item_id = ?1 ORDER BY id",
            )?;
            let rows = statement.query_map([id], |row| {
                Ok(ImageRef {
                    kind: match row.get::<_, String>(0)?.as_str() {
                        "fanart" => ImageKind::Fanart,
                        "banner" => ImageKind::Banner,
                        "season" => ImageKind::SeasonPoster,
                        "episode_thumb" => ImageKind::EpisodeThumb,
                        "actor" => ImageKind::Actor,
                        "logo" => ImageKind::Logo,
                        "clearart" => ImageKind::ClearArt,
                        _ => ImageKind::Poster,
                    },
                    season: row.get::<_, Option<i64>>(1)?.map(|n| n as u32),
                    source_url: row.get(2)?,
                    local_path: PathBuf::from(row.get::<_, String>(3)?),
                    language: row.get(4)?,
                })
            })?;
            item.images = rows.collect::<Result<Vec<_>, _>>()?;
        }

        {
            let mut statement = self.conn.prepare(
                "SELECT path, size, mtime, stack_part, is_extra FROM media_file WHERE item_id = ?1 ORDER BY id",
            )?;
            let rows = statement.query_map([id], |row| {
                Ok(MediaFileRef {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    size: row.get::<_, Option<i64>>(1)?.map(|n| n as u64),
                    mtime: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
                    stack_part: row.get::<_, Option<i64>>(3)?.map(|n| n as u32),
                    is_extra: row.get::<_, i64>(4)? != 0,
                })
            })?;
            item.files = rows.collect::<Result<Vec<_>, _>>()?;
        }

        Ok(())
    }
}

fn parse_kind(value: &str) -> rusqlite::Result<MediaKind> {
    match value {
        "movie" => Ok(MediaKind::Movie),
        "tvshow" => Ok(MediaKind::TvShow),
        "episode" => Ok(MediaKind::Episode),
        "concert" => Ok(MediaKind::Concert),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown media kind: {other}"),
            )),
        )),
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn sample_item(title: &str) -> MediaItem {
        let mut item = MediaItem::new(
            MediaKind::Movie,
            PathBuf::from("/movies/Alien"),
            PathBuf::from("/movies/Alien/a.mkv"),
            title.to_owned(),
        );
        item.year = Some(1979);
        item.overview = Some("A crew encounters a hostile alien.".to_owned());
        item.ids.imdb = Some("tt0078748".to_owned());
        item.ratings = vec![Rating {
            source: "imdb".to_owned(),
            value: 8.5,
            votes: 1000,
            max: 10.0,
            is_default: true,
        }];
        item.genres = vec!["Horror".to_owned(), "Sci-Fi".to_owned()];
        item.credits.push(Credit {
            kind: CreditKind::Actor,
            name: "Sigourney Weaver".to_owned(),
            role: Some("Ripley".to_owned()),
            thumb_url: None,
            ordering: 0,
        });
        item.images.push(ImageRef {
            kind: ImageKind::Poster,
            season: None,
            source_url: None,
            local_path: PathBuf::from("/movies/Alien/poster.jpg"),
            language: None,
        });
        item.files.push(MediaFileRef {
            path: PathBuf::from("/movies/Alien/a.mkv"),
            size: Some(1234),
            mtime: Some(5678),
            stack_part: None,
            is_extra: false,
        });
        item
    }

    #[test]
    fn upsert_then_get_round_trips_all_fields() {
        let mut db = Database::open_in_memory().unwrap();
        let mut repo = MediaRepository::new(db.connection_mut());
        let id = repo.upsert(&sample_item("Alien")).unwrap();

        let loaded = repo.get(id).unwrap().unwrap();
        assert_eq!(loaded.title, "Alien");
        assert_eq!(loaded.year, Some(1979));
        assert_eq!(loaded.ids.imdb.as_deref(), Some("tt0078748"));
        assert_eq!(loaded.genres, vec!["Horror", "Sci-Fi"]);
        assert_eq!(loaded.ratings.len(), 1);
        assert_eq!(loaded.ratings[0].source, "imdb");
        assert_eq!(loaded.credits[0].name, "Sigourney Weaver");
        assert_eq!(loaded.images[0].kind, ImageKind::Poster);
        assert_eq!(loaded.files[0].size, Some(1234));
    }

    #[test]
    fn upsert_is_idempotent_for_the_same_primary_file() {
        let mut db = Database::open_in_memory().unwrap();
        let mut repo = MediaRepository::new(db.connection_mut());
        let first = repo.upsert(&sample_item("Old title")).unwrap();
        let second = repo.upsert(&sample_item("New title")).unwrap();

        assert_eq!(first, second);
        assert_eq!(repo.count(None, None).unwrap(), 1);
        assert_eq!(repo.get(second).unwrap().unwrap().title, "New title");
    }

    #[test]
    fn list_filters_by_kind_and_search_term() {
        let mut db = Database::open_in_memory().unwrap();
        let mut repo = MediaRepository::new(db.connection_mut());
        repo.upsert(&sample_item("Alien")).unwrap();

        let mut show = sample_item("Torchwood");
        show.kind = MediaKind::TvShow;
        show.primary_file = PathBuf::from("/shows/Torchwood/tvshow.nfo");
        show.overview = Some("A Welsh science fiction series.".to_owned());
        show.files.clear();
        repo.upsert(&show).unwrap();

        assert_eq!(
            repo.list(Some(MediaKind::Movie), None, 50, 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(repo.list(None, Some("alien"), 50, 0).unwrap().len(), 1);
        assert_eq!(repo.count(Some(MediaKind::Episode), None).unwrap(), 0);
        assert_eq!(repo.count(None, Some("missing")).unwrap(), 0);
    }

    #[test]
    fn scan_roots_can_be_stored_and_updated() {
        let mut db = Database::open_in_memory().unwrap();
        let repo = MediaRepository::new(db.connection_mut());
        let root = Path::new("/media/movies");

        repo.upsert_root(root, "movie", true).unwrap();
        repo.touch_root(root).unwrap();
        let roots = repo.list_roots().unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, root);
        assert_eq!(roots[0].kind, "movie");
        assert!(roots[0].recursive);
        assert!(roots[0].last_scan.is_some());
    }

    #[test]
    fn clear_items_removes_children_via_cascade() {
        let mut db = Database::open_in_memory().unwrap();
        {
            let mut repo = MediaRepository::new(db.connection_mut());
            repo.upsert(&sample_item("Alien")).unwrap();
            repo.clear_items().unwrap();
            assert_eq!(repo.count(None, None).unwrap(), 0);
        }
        let credits: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM credit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(credits, 0);
    }
}
