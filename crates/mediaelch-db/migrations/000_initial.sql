-- Scan roots and the central media item table.
CREATE TABLE media_directory (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL DEFAULT 'auto',
    recursive INTEGER NOT NULL DEFAULT 1,
    last_scan TEXT
);

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

CREATE INDEX ix_item_kind ON media_item(kind);
CREATE INDEX ix_item_parent ON media_item(parent_id);
CREATE INDEX ix_item_title ON media_item(sort_title);
