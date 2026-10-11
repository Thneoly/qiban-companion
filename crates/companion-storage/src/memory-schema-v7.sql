-- v7: finalize the send-audit dispatch log shape. v6 created the table but
-- nothing writes it yet, so it is dropped and recreated (empty by
-- construction): the prompt column makes cancelled/failed requests fully
-- reconstructable too (they leave no turn row), and history_turns replaces
-- the id range — the 6-turn global eviction means ids do not survive anyway,
-- the honest record is how many turns rode the request.
DROP TABLE chat_dispatch_log;
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
    history_turns INTEGER NOT NULL DEFAULT 0,
    memory_count INTEGER NOT NULL DEFAULT 0,
    memory_chars INTEGER NOT NULL DEFAULT 0,
    prompt TEXT NOT NULL
);
PRAGMA user_version=7;
