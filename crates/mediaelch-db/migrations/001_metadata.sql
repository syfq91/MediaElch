-- Files, stream details, credits, tags and artwork.
CREATE TABLE media_file (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    size INTEGER,
    mtime INTEGER,
    stack_part INTEGER,
    is_extra INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX ix_file_item ON media_file(item_id);

CREATE TABLE stream_detail (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES media_file(id) ON DELETE CASCADE,
    stream_type TEXT NOT NULL,
    codec TEXT,
    resolution TEXT,
    channels INTEGER,
    language TEXT
);

CREATE INDEX ix_stream_file ON stream_detail(file_id);

CREATE TABLE credit (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT,
    thumb_url TEXT,
    ordering INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX ix_credit_item ON credit(item_id);

CREATE TABLE media_tag (
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (item_id, kind, value)
);

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

CREATE INDEX ix_image_item ON image(item_id);
