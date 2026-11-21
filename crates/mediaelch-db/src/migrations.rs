//! Ordered, embedded SQL migrations applied on every database open.

use std::collections::HashSet;

use rusqlite::{params, Connection};

use crate::DbError;

/// One migration, embedded into the binary at compile time.
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 0,
        name: "initial",
        sql: include_str!("../migrations/000_initial.sql"),
    },
    Migration {
        version: 1,
        name: "metadata",
        sql: include_str!("../migrations/001_metadata.sql"),
    },
    Migration {
        version: 2,
        name: "scrape_settings_fts",
        sql: include_str!("../migrations/002_scrape_settings_fts.sql"),
    },
    Migration {
        version: 3,
        name: "ratings",
        sql: include_str!("../migrations/003_ratings.sql"),
    },
];

/// Apply every migration that has not been recorded yet. Each migration runs
/// in its own transaction, so a failure rolls back only that step.
pub fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (datetime('now'))
         )",
    )?;

    let applied: HashSet<u32> = {
        let mut statement = conn.prepare("SELECT version FROM schema_migrations")?;
        let rows = statement.query_map([], |row| row.get::<_, u32>(0))?;
        rows.collect::<Result<_, _>>()?
    };

    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }

        let fail = |source| DbError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        };
        let transaction = conn.transaction().map_err(fail)?;
        transaction.execute_batch(migration.sql).map_err(fail)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )
            .map_err(fail)?;
        transaction.commit().map_err(fail)?;

        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applied database migration"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// Stable, human-readable form of the resulting schema. FTS5 shadow tables
    /// are omitted because they are an implementation detail of the index.
    fn canonical_schema(conn: &Connection) -> String {
        let mut statement = conn
            .prepare(
                "SELECT type, name, sql FROM sqlite_master
                 WHERE sql IS NOT NULL
                   AND name NOT LIKE 'sqlite_%'
                   AND NOT (type = 'table' AND name LIKE 'item_fts_%')
                   AND name != 'schema_migrations'
                 ORDER BY type, name",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap();

        let mut out = String::new();
        for row in rows {
            let (kind, name, sql) = row.unwrap();
            out.push_str(&format!("-- {kind} {name}\n{sql};\n"));
        }
        out
    }

    fn migration_count(db: &Database) -> i64 {
        db.connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn migrations_produce_the_expected_schema() {
        let db = Database::open_in_memory().unwrap();
        let schema = canonical_schema(db.connection());
        let snapshot = include_str!("../schema.sql");

        if std::env::var_os("MEDIAELCH_UPDATE_SCHEMA").is_some() {
            std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/schema.sql"), &schema).unwrap();
        }

        assert_eq!(schema.trim(), snapshot.trim());
        assert_eq!(migration_count(&db), MIGRATIONS.len() as i64);
    }

    #[test]
    fn reopening_a_database_does_not_reapply_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mediaelch.sqlite");

        let first = Database::open(&path).unwrap();
        assert_eq!(migration_count(&first), MIGRATIONS.len() as i64);

        let second = Database::open(&path).unwrap();
        assert_eq!(migration_count(&second), MIGRATIONS.len() as i64);
    }

    #[test]
    fn full_text_search_index_is_usable() {
        let db = Database::open_in_memory().unwrap();
        db.connection()
            .execute(
                "INSERT INTO media_item (kind, directory, primary_file, title)
                 VALUES ('movie', '/movies', '/movies/a.mkv', 'The Matrix')",
                [],
            )
            .unwrap();

        let matches: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM item_fts WHERE item_fts MATCH 'matrix'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matches, 1);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Database::open_in_memory().unwrap();
        let result = db.connection().execute(
            "INSERT INTO media_file (item_id, path) VALUES (999, '/tmp/orphan.mkv')",
            [],
        );
        assert!(result.is_err());
    }
}
