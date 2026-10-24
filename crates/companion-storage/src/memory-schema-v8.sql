-- v8: local search (read-only) support. The authorized_roots table is the
-- standing user consent (survives chat_clear — it is removed only through
-- the settings command); the tool_invocation_log is request content: every
-- tool result that was replayed into a model request lands one row with
-- the EXACT payload sent (8KB-capped excerpt), so "what file content left
-- this machine" is answerable locally. The dispatch log gains the tool
-- round count and any intermediate assistant text that rode the requests.
CREATE TABLE authorized_roots (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    label TEXT,
    added_at INTEGER NOT NULL
);
CREATE TABLE tool_invocation_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    round INTEGER NOT NULL DEFAULT 0,
    tool TEXT NOT NULL,
    arguments TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('ok','denied','error','budget','unknown_tool')),
    result_chars INTEGER NOT NULL DEFAULT 0,
    result_excerpt TEXT,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    occurred_at INTEGER NOT NULL
);
ALTER TABLE chat_dispatch_log ADD COLUMN tool_rounds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chat_dispatch_log ADD COLUMN intermediate_text TEXT;
PRAGMA user_version=8;
