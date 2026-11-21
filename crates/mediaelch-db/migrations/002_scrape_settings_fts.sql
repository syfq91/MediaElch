-- Scraper state, typed settings and full-text search over media items.
CREATE TABLE scrape_state (
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_id TEXT,
    last_scrape TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (item_id, provider)
);

CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE VIRTUAL TABLE item_fts USING fts5(
    title,
    original_title,
    overview,
    content = 'media_item',
    content_rowid = 'id'
);

CREATE TRIGGER item_fts_insert AFTER INSERT ON media_item BEGIN
    INSERT INTO item_fts(rowid, title, original_title, overview)
    VALUES (new.id, new.title, new.original_title, new.overview);
END;

CREATE TRIGGER item_fts_delete AFTER DELETE ON media_item BEGIN
    INSERT INTO item_fts(item_fts, rowid, title, original_title, overview)
    VALUES ('delete', old.id, old.title, old.original_title, old.overview);
END;

CREATE TRIGGER item_fts_update AFTER UPDATE ON media_item BEGIN
    INSERT INTO item_fts(item_fts, rowid, title, original_title, overview)
    VALUES ('delete', old.id, old.title, old.original_title, old.overview);
    INSERT INTO item_fts(rowid, title, original_title, overview)
    VALUES (new.id, new.title, new.original_title, new.overview);
END;
