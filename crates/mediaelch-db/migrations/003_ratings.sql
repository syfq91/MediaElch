-- Individual ratings from metadata providers (imdb, themoviedb, ...).
CREATE TABLE rating (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    value REAL NOT NULL,
    votes INTEGER NOT NULL DEFAULT 0,
    max_value REAL NOT NULL DEFAULT 10,
    is_default INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX ix_rating_item ON rating(item_id);
