-- Personal memory injection policy (schema v3, additive over v2).
-- Rows reference memories in the standalone memory service by integer id;
-- there is deliberately no FK into `memories` (different id domain) and no
-- local cascade: service-side forget/supersede produce no local event, and
-- drift is caught at send time by the (id, seq) admission check.
CREATE TABLE personal_memory_policy (
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
    PRIMARY KEY(base_url,model)
);
CREATE TABLE personal_memory_selection (
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    personal_memory_id INTEGER NOT NULL CHECK(personal_memory_id >= 1),
    position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 4),
    PRIMARY KEY(base_url,model,personal_memory_id),
    UNIQUE(base_url,model,position),
    FOREIGN KEY(base_url,model) REFERENCES personal_memory_policy(base_url,model)
);
PRAGMA user_version=3;
