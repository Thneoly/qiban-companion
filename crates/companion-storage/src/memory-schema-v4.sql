-- Chat usage ledger (schema v4, additive over v3): which memory items each
-- persisted turn actually carried into the provider request. Drives precise
-- chat cleanup (M4 fix): editing/deleting a memory prunes only the turns that
-- used it. Deliberately NO FK into `memories`: cleanup must still see the id
-- after the memory row is scrubbed to a tombstone, and personal ids live in
-- the external memory service (v3 precedent). Turn eviction and manual
-- clears cascade through chat_turns.
CREATE TABLE chat_turn_usage (
    turn_id INTEGER NOT NULL REFERENCES chat_turns(id) ON DELETE CASCADE,
    memory_kind TEXT NOT NULL CHECK(memory_kind IN ('app','personal')),
    memory_id TEXT,
    revision INTEGER,
    personal_id INTEGER,
    personal_seq INTEGER,
    CHECK((memory_kind='app' AND memory_id IS NOT NULL
             AND revision BETWEEN 1 AND 9007199254740991
             AND personal_id IS NULL AND personal_seq IS NULL)
       OR (memory_kind='personal' AND memory_id IS NULL AND revision IS NULL
             AND personal_id >= 1 AND (personal_seq IS NULL OR personal_seq >= 1)))
);
-- NULLs never equal in SQLite unique constraints; the expression index is
-- the real uniqueness guarantee. Callers dedupe as well.
CREATE UNIQUE INDEX chat_turn_usage_unique
  ON chat_turn_usage(turn_id, memory_kind, coalesce(memory_id,''), coalesce(personal_id,0));
CREATE INDEX chat_turn_usage_lookup
  ON chat_turn_usage(memory_kind, coalesce(memory_id,''), coalesce(personal_id,0));
-- Backfill: attribute pre-v4 turns to the selections enabled at migration
-- time. Conservative direction (over-attribution clears more, never less);
-- turns whose selections changed mid-history may be missed — that blind spot
-- is documented in docs/status/memory-precise-clear.md. personal_seq is NULL:
-- the send-time seq of a past turn is not recoverable.
INSERT INTO chat_turn_usage(turn_id,memory_kind,memory_id,revision,personal_id,personal_seq)
SELECT t.id,'app',s.memory_id,m.revision,NULL,NULL
FROM chat_turns t
JOIN memory_policy p ON p.base_url=t.base AND p.model=t.model AND p.enabled=1
JOIN memory_selection s ON s.base_url=t.base AND s.model=t.model
JOIN memories m ON m.id=s.memory_id AND m.deleted_at IS NULL;
INSERT INTO chat_turn_usage(turn_id,memory_kind,memory_id,revision,personal_id,personal_seq)
SELECT t.id,'personal',NULL,NULL,s.personal_memory_id,NULL
FROM chat_turns t
JOIN personal_memory_policy p ON p.base_url=t.base AND p.model=t.model AND p.enabled=1
JOIN personal_memory_selection s ON s.base_url=t.base AND s.model=t.model;
PRAGMA user_version=4;
