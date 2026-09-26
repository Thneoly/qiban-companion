-- Personal memory schema v2, evolved in place from the Python-era v1 layout.
-- Timestamps stay TEXT UTC "YYYY-MM-DD HH:MM:SS" so plain string comparison
-- against datetime('now') keeps the v1 semantics byte-for-byte.
CREATE TABLE memory_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    next_seq INTEGER NOT NULL CHECK(next_seq BETWEEN 1 AND 9007199254740991)
);
INSERT INTO memory_meta VALUES(1,1);
CREATE TABLE memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    seq INTEGER NOT NULL CHECK(seq BETWEEN 1 AND 9007199254740991),
    type TEXT NOT NULL CHECK(type IN ('fact','decision','preference','project','person','insight','context')),
    project TEXT,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 200),
    content TEXT NOT NULL CHECK(length(content) BETWEEN 1 AND 20000),
    importance INTEGER NOT NULL DEFAULT 3 CHECK(importance BETWEEN 1 AND 5),
    created_at TEXT NOT NULL CHECK(created_at GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-5][0-9]'),
    updated_at TEXT NOT NULL CHECK(updated_at GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-5][0-9]'),
    valid_until TEXT CHECK(valid_until IS NULL OR valid_until GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-5][0-9]'),
    superseded_by INTEGER REFERENCES memories(id),
    contradicts INTEGER REFERENCES memories(id),
    tags TEXT NOT NULL DEFAULT '',
    origin TEXT CHECK(origin IS NULL OR (length(origin) BETWEEN 1 AND 32 AND origin NOT GLOB '*[^a-z0-9_-]*')),
    UNIQUE(seq),
    CHECK(superseded_by IS NULL OR superseded_by <> id),
    CHECK(contradicts IS NULL OR contradicts <> id)
);
CREATE INDEX idx_memories_active ON memories(superseded_by, valid_until, importance, updated_at);
CREATE INDEX idx_memories_project ON memories(project);
CREATE INDEX idx_memories_type ON memories(type);
PRAGMA user_version=2;
