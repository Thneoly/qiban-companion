-- v6: task facts, send-audit dispatch log and reminders.
-- The memories row-level CHECK widens for active task_fact rows
-- (source_kind='executor'); manual rows keep their exact shape. SQLite cannot
-- ALTER a table CHECK, so memories and its FK-referencing memory_selection
-- are rebuilt in the standard rename dance (selection's FK ends up pointing
-- at the rebuilt memories by the rename rewrite). Tombstone rows are copied
-- verbatim and their branch of the CHECK is unchanged.
CREATE TABLE memories_v6 (
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
    CHECK((deleted_at IS NULL
        AND body IS NOT NULL AND length(body) BETWEEN 1 AND 200
        AND created_at IS NOT NULL AND created_at>=0
        AND confirmed_at IS NOT NULL AND confirmed_at>=created_at
        AND updated_at IS NOT NULL AND updated_at=confirmed_at
        AND ((source_kind='user_manual' AND source_label IS NOT NULL
              AND source_label='用户在记忆面板填写'
              AND kind IN ('preference','experience'))
          OR (source_kind='executor' AND source_label IS NOT NULL
              AND source_label='来自电脑上的文档任务'
              AND kind='task_fact')))
      OR (deleted_at IS NOT NULL AND deleted_at>=0 AND body IS NULL
        AND source_kind IS NULL AND source_label IS NULL AND event_date IS NULL
        AND created_at IS NULL AND confirmed_at IS NULL AND updated_at IS NULL))
);
INSERT INTO memories_v6 SELECT * FROM memories;
CREATE TABLE memory_selection_v6 (
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    memory_id TEXT NOT NULL REFERENCES memories_v6(id),
    position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 4),
    PRIMARY KEY(base_url,model,memory_id),
    UNIQUE(base_url,model,position),
    FOREIGN KEY(base_url,model) REFERENCES memory_policy(base_url,model)
);
INSERT INTO memory_selection_v6 SELECT * FROM memory_selection;
DROP TABLE memory_selection;
DROP TABLE memories;
ALTER TABLE memories_v6 RENAME TO memories;
ALTER TABLE memory_selection_v6 RENAME TO memory_selection;
-- task <-> fact identity: one ACTIVE fact row per task fingerprint; redo
-- retargets this pointer and tombstones the previous row.
CREATE TABLE task_fact_index (
    task_fingerprint TEXT PRIMARY KEY NOT NULL,
    memory_id TEXT NOT NULL REFERENCES memories(id)
);
-- Send-audit invariant (model-visible means logged). Independent lifecycle:
-- a ring, not tied to turn rows (cancelled/failed requests have no turn).
CREATE TABLE chat_dispatch_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE,
    outcome TEXT NOT NULL CHECK(outcome IN ('sent','cancelled','timeout','dropped')),
    model TEXT NOT NULL,
    base_url TEXT NOT NULL,
    dispatched_at INTEGER NOT NULL,
    system_version INTEGER NOT NULL,
    memory_block TEXT,
    personal_digest TEXT,
    history_from INTEGER,
    history_to INTEGER,
    memory_count INTEGER NOT NULL DEFAULT 0,
    memory_chars INTEGER NOT NULL DEFAULT 0
);
-- Reminders: user-created, app-local only.
CREATE TABLE reminders (
    id TEXT PRIMARY KEY NOT NULL,
    due_at INTEGER NOT NULL CHECK(due_at BETWEEN 0 AND 9007199254740991),
    message TEXT NOT NULL CHECK(length(message) BETWEEN 1 AND 200),
    status TEXT NOT NULL CHECK(status IN ('pending','fired','cancelled'))
);
PRAGMA user_version=6;
