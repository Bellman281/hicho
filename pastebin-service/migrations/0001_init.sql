-- Pastes table. `id` is the primary key (unique index for free; a duplicate
-- insert becomes a constraint violation mapped to Conflict). `syntax` and
-- `expires_at` are nullable; `one_shot` is a 0/1 flag (SQLite has no bool).
CREATE TABLE IF NOT EXISTS pastes (
    id         TEXT    NOT NULL PRIMARY KEY,
    content    TEXT    NOT NULL,
    syntax     TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    one_shot   INTEGER NOT NULL DEFAULT 0,
    views      INTEGER NOT NULL DEFAULT 0
);
