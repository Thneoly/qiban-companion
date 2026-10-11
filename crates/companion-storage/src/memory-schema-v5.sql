-- v5: cross-device sync bookkeeping on the desktop side.
-- memory_outbox lives in the SAME database as the memories table so enqueue
-- is part of the mutation transaction (device-continuity: outbox events and
-- business changes commit together or not at all). Rows are owner-neutral:
-- the server base revision is resolved against the current owner's mirror at
-- push time, which is what makes an account switch a merge instead of a fork.
-- Folding rewrites the request id so an in-flight push of the old payload
-- can never ack away the folded-in newer change.
CREATE TABLE memory_outbox (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  request_id TEXT NOT NULL UNIQUE,
  op TEXT NOT NULL CHECK(op IN ('create','update','delete')),
  memory_id TEXT NOT NULL,
  kind TEXT,
  body TEXT,
  event_date TEXT,
  created_at INTEGER NOT NULL);
CREATE INDEX memory_outbox_order ON memory_outbox(seq);
CREATE TABLE memory_sync_owners (
  owner TEXT PRIMARY KEY,
  generation INTEGER NOT NULL DEFAULT 0,
  cursor INTEGER NOT NULL DEFAULT 0);
CREATE TABLE memory_server_mirror (
  owner TEXT NOT NULL,
  memory_id TEXT NOT NULL,
  server_revision INTEGER NOT NULL,
  PRIMARY KEY(owner,memory_id));
PRAGMA user_version=5;
