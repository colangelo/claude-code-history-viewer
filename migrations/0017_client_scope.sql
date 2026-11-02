-- Client scope (openspec/changes/hub-client-read-scope, design D1, R2-B, R2-C, R2-D, R2-I).
--
-- A session is LAB only when `client_class = 'none'` AND `client_gen` is the
-- hub's current policy generation. Everything else (NULL class, NULL or older
-- generation, any other class) is client for a lab reader: unscanned means client.
--
-- `client_class` only rises: from NULL or 'none' to a client class, never back,
-- except through the audited `client-scope clear` (written to client_scope_audit).
-- `client_rev` is bumped by every ingest batch that touches the session (its row
-- or its messages). The sweep stamps `client_gen` only when `client_rev` is still
-- the value it read, so an ingest racing a sweep leaves the session unscanned.
ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS client_class TEXT,
    ADD COLUMN IF NOT EXISTS client_gen   TEXT,
    ADD COLUMN IF NOT EXISTS client_rev   BIGINT NOT NULL DEFAULT 0;

-- The sweep walks unscanned sessions; lab reads filter on the pair.
CREATE INDEX IF NOT EXISTS sessions_client_scope_idx ON sessions (client_gen, client_class);

-- The accepted marker set: one sha256 per normalized marker, never the marker.
-- A generation that adds markers is accepted on load; one that removes or
-- replaces any needs `client-scope accept-routing` (R2-D).
CREATE TABLE IF NOT EXISTS client_scope_policy (
    marker_hash  TEXT        PRIMARY KEY,
    accepted_gen TEXT        NOT NULL,
    accepted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_by  TEXT        NOT NULL
);

-- One row: the generation in force, and the enforcement latch (R2-I).
CREATE TABLE IF NOT EXISTS client_scope_state (
    singleton   BOOLEAN     PRIMARY KEY DEFAULT true CHECK (singleton),
    generation  TEXT,
    enforced_at TIMESTAMPTZ,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO client_scope_state (singleton) VALUES (true) ON CONFLICT DO NOTHING;

-- Every audited client-scope action: accept-routing, clear, unlatch,
-- restamp-history, and every policy generation change.
CREATE TABLE IF NOT EXISTS client_scope_audit (
    id          BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    action      TEXT        NOT NULL,
    session_id  BIGINT,
    entry_id    BIGINT,
    generation  TEXT,
    detail      TEXT,
    approved_by TEXT
);

-- restamp-history (R2-F) writes a route the old check did not allow.
ALTER TABLE journal_entries DROP CONSTRAINT IF EXISTS journal_entries_route_check;
ALTER TABLE journal_entries
    ADD CONSTRAINT journal_entries_route_check
    CHECK (route IN ('general', 'protected', 'general-restamped'));
