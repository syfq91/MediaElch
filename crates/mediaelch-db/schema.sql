-- index ix_credit_item
CREATE INDEX ix_credit_item ON credit(item_id);
-- index ix_file_item
CREATE INDEX ix_file_item ON media_file(item_id);
-- index ix_image_item
CREATE INDEX ix_image_item ON image(item_id);
-- index ix_item_kind
CREATE INDEX ix_item_kind ON media_item(kind);
-- index ix_item_parent
CREATE INDEX ix_item_parent ON media_item(parent_id);
-- index ix_item_title
CREATE INDEX ix_item_title ON media_item(sort_title);
-- index ix_stream_file
CREATE INDEX ix_stream_file ON stream_detail(file_id);
-- table credit
CREATE TABLE credit (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT,
    thumb_url TEXT,
    ordering INTEGER NOT NULL DEFAULT 0
);
-- table image
CREATE TABLE image (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    season INTEGER,
    source_url TEXT,
    local_path TEXT NOT NULL,
    language TEXT,
    width INTEGER,
    height INTEGER
);
-- table item_fts
CREATE VIRTUAL TABLE item_fts USING fts5(
    title,
    original_title,
    overview,
    content = 'media_item',
    content_rowid = 'id'
);
-- table media_directory
CREATE TABLE media_directory (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL DEFAULT 'auto',
    recursive INTEGER NOT NULL DEFAULT 1,
    last_scan TEXT
);
-- table media_file
CREATE TABLE media_file (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    size INTEGER,
    mtime INTEGER,
    stack_part INTEGER,
    is_extra INTEGER NOT NULL DEFAULT 0
);
-- table media_item
CREATE TABLE media_item (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('movie', 'tvshow', 'episode', 'concert')),
    parent_id INTEGER REFERENCES media_item(id) ON DELETE CASCADE,
    directory TEXT NOT NULL,
    primary_file TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    sort_title TEXT,
    original_title TEXT,
    year INTEGER,
    released TEXT,
    overview TEXT,
    runtime_min INTEGER,
    certification TEXT,
    rating REAL,
    votes INTEGER,
    rating_source TEXT,
    imdb_id TEXT,
    tmdb_id INTEGER,
    tvmaze_id INTEGER,
    season_number INTEGER,
    episode_number INTEGER,
    nfo_path TEXT,
    nfo_mtime INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
-- table media_tag
CREATE TABLE media_tag (
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (item_id, kind, value)
);
-- table scrape_state
CREATE TABLE scrape_state (
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_id TEXT,
    last_scrape TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (item_id, provider)
);
-- table setting
CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- table stream_detail
CREATE TABLE stream_detail (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES media_file(id) ON DELETE CASCADE,
    stream_type TEXT NOT NULL,
    codec TEXT,
    resolution TEXT,
    channels INTEGER,
    language TEXT
);
-- trigger item_fts_delete
CREATE TRIGGER item_fts_delete AFTER DELETE ON media_item BEGIN
    INSERT INTO item_fts(item_fts, rowid, title, original_title, overview)
    VALUES ('delete', old.id, old.title, old.original_title, old.overview);
END;
-- trigger item_fts_insert
CREATE TRIGGER item_fts_insert AFTER INSERT ON media_item BEGIN
    INSERT INTO item_fts(rowid, title, original_title, overview)
    VALUES (new.id, new.title, new.original_title, new.overview);
END;
-- trigger item_fts_update
CREATE TRIGGER item_fts_update AFTER UPDATE ON media_item BEGIN
    INSERT INTO item_fts(item_fts, rowid, title, original_title, overview)
    VALUES ('delete', old.id, old.title, old.original_title, old.overview);
    INSERT INTO item_fts(rowid, title, original_title, overview)
    VALUES (new.id, new.title, new.original_title, new.overview);
END;
