CREATE TABLE memory_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    context_epoch INTEGER NOT NULL CHECK(context_epoch BETWEEN 0 AND 9007199254740991)
);
INSERT INTO memory_meta VALUES(1,0);
CREATE TABLE memories (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('preference','experience','task_fact')),
    body TEXT,
    source_kind TEXT,
    source_label TEXT,
    event_date TEXT,
    created_at INTEGER,
    confirmed_at INTEGER,
    updated_at INTEGER,
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    deleted_at INTEGER,
    CHECK((deleted_at IS NULL AND kind IN ('preference','experience')
        AND body IS NOT NULL AND length(body) BETWEEN 1 AND 200
        AND source_kind IS NOT NULL AND source_kind='user_manual'
        AND source_label IS NOT NULL AND source_label='用户在记忆面板填写'
        AND created_at IS NOT NULL AND created_at>=0
        AND confirmed_at IS NOT NULL AND confirmed_at>=created_at
        AND updated_at IS NOT NULL AND updated_at=confirmed_at)
      OR (deleted_at IS NOT NULL AND deleted_at>=0 AND body IS NULL
        AND source_kind IS NULL AND source_label IS NULL AND event_date IS NULL
        AND created_at IS NULL AND confirmed_at IS NULL AND updated_at IS NULL))
);
CREATE TABLE memory_policy (
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
    PRIMARY KEY(base_url,model)
);
CREATE TABLE memory_selection (
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    memory_id TEXT NOT NULL REFERENCES memories(id),
    position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 4),
    PRIMARY KEY(base_url,model,memory_id),
    UNIQUE(base_url,model,position),
    FOREIGN KEY(base_url,model) REFERENCES memory_policy(base_url,model)
);
PRAGMA user_version=2;
