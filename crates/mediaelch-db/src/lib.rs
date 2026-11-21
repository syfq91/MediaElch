//! SQLite storage for the headless MediaElch server.
//!
//! The database is created from scratch with our own schema; the legacy
//! `MediaElch.sqlite` cache of the Qt application is never opened.

pub mod migrations;

use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("failed to open database at {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("migration {name} ({version}) failed: {source}")]
    Migration {
        version: u32,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },
}

/// An open connection to the server-owned SQLite database.
pub struct Database {
    conn: rusqlite::Connection,
}

impl Database {
    /// Open (or create) a database file and apply the connection pragmas used
    /// throughout the server: WAL journaling, foreign keys and a busy timeout.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref();
        let conn = rusqlite::Connection::open(path).map_err(|source| DbError::Open {
            path: path.display().to_string(),
            source,
        })?;
        let db = Self::from_connection(conn)?;
        tracing::info!(path = %path.display(), "opened media database");
        Ok(db)
    }

    /// Create an in-memory database, mainly for tests.
    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::from_connection(rusqlite::Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: rusqlite::Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        migrations::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &rusqlite::Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_new_file_enables_the_expected_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mediaelch.sqlite");

        let db = Database::open(&path).unwrap();

        assert!(path.exists());
        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");
        let foreign_keys: i64 = db
            .connection()
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn in_memory_database_is_usable() {
        let db = Database::open_in_memory().unwrap();
        db.connection()
            .execute("CREATE TABLE ping (value TEXT)", [])
            .unwrap();
    }
}
