-- history-archive-mvp :: initial schema
--
-- Durable, cross-machine archive of normalized AI coding-agent history.
-- Owned and applied exclusively by the hub service (the only component with
-- Postgres credentials). Designed to be pgvector-ready WITHOUT requiring the
-- `vector` extension now: embeddings will live in a separate `message_embeddings`
-- table added in a later migration (see the note at the end of this file).
--
-- Cumulative archive semantic: rows are written once and never deleted by the
-- sync daemon. Foreign keys intentionally use the default NO ACTION (not
-- CASCADE) so an accidental session delete cannot cascade away archived
-- messages.

-- ---------------------------------------------------------------------------
-- machines: one row per machine that pushes history (task 2.2)
-- ---------------------------------------------------------------------------
CREATE TABLE machines (
    machine_id  UUID        PRIMARY KEY,
    hostname    TEXT        NOT NULL,
    os          TEXT,
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- projects: a provider project/workspace, scoped to a machine (task 2.3)
-- ---------------------------------------------------------------------------
CREATE TABLE projects (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    machine_id     UUID        NOT NULL REFERENCES machines (machine_id),
    provider       TEXT        NOT NULL,
    project_path   TEXT        NOT NULL,
    name           TEXT,
    storage_type   TEXT,
    -- maintained aggregates so browse queries never scan all messages
    session_count  INTEGER     NOT NULL DEFAULT 0,
    message_count  INTEGER     NOT NULL DEFAULT 0,
    last_modified  TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (machine_id, provider, project_path)
);

-- ---------------------------------------------------------------------------
-- sessions: a single conversation/session within a project (task 2.4)
-- ---------------------------------------------------------------------------
CREATE TABLE sessions (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    machine_id          UUID        NOT NULL REFERENCES machines (machine_id),
    provider            TEXT        NOT NULL,
    session_id          TEXT        NOT NULL, -- the provider's own session id
    project_id          BIGINT      REFERENCES projects (id),
    file_path           TEXT,
    entrypoint          TEXT,
    summary             TEXT,
    message_count       INTEGER     NOT NULL DEFAULT 0,
    first_message_time  TIMESTAMPTZ,
    last_message_time   TIMESTAMPTZ,
    last_modified       TIMESTAMPTZ,
    has_tool_use        BOOLEAN     NOT NULL DEFAULT false,
    has_errors          BOOLEAN     NOT NULL DEFAULT false,
    storage_type        TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (machine_id, provider, session_id)
);

-- ---------------------------------------------------------------------------
-- messages: normalized message + raw-fidelity original + FTS (task 2.5)
--
-- Dedup key: (session_id, message_key). The session surrogate FK already
-- encodes (machine_id, provider, provider_session_id), so this enforces the
-- spec's logical identity (machine_id, provider, session_id, message_key)
-- in normalized form. `message_key` is the provider message UUID when present,
-- otherwise a content-derived key. machine_id/provider are denormalized here
-- so search can filter without joining sessions.
-- ---------------------------------------------------------------------------
CREATE TABLE messages (
    id                     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    session_id             BIGINT      NOT NULL REFERENCES sessions (id),
    machine_id             UUID        NOT NULL REFERENCES machines (machine_id),
    provider               TEXT        NOT NULL,
    message_key            TEXT        NOT NULL,
    uuid                   TEXT,
    parent_uuid            TEXT,
    seq                    INTEGER     NOT NULL DEFAULT 0,
    "timestamp"            TIMESTAMPTZ,
    type                   TEXT,
    role                   TEXT,
    model                  TEXT,
    stop_reason            TEXT,
    input_tokens           BIGINT,
    output_tokens          BIGINT,
    cache_creation_tokens  BIGINT,
    cache_read_tokens      BIGINT,
    cost_usd               DOUBLE PRECISION,
    duration_ms            BIGINT,
    is_sidechain           BOOLEAN     NOT NULL DEFAULT false,
    content                JSONB,                 -- normalized content
    raw                    JSONB       NOT NULL,  -- EXACT original record (fidelity/reprocessing)
    search_text            TEXT,                  -- flattened plaintext (built by history-core)
    text_search            TSVECTOR
        GENERATED ALWAYS AS (to_tsvector('simple', coalesce(search_text, ''))) STORED,
    content_hash           BYTEA,                 -- integrity / future re-dedup
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, message_key)
);

-- ---------------------------------------------------------------------------
-- indexes (task 2.6)
-- ---------------------------------------------------------------------------
-- Full-text search over message content.
CREATE INDEX messages_text_search_idx ON messages USING GIN (text_search);
-- Search/browse filters by machine + provider without joining sessions.
CREATE INDEX messages_machine_provider_idx ON messages (machine_id, provider);
-- Time-range filters and chronological ordering.
CREATE INDEX messages_timestamp_idx ON messages ("timestamp");
-- "sessions of a project" browse.
CREATE INDEX sessions_project_id_idx ON sessions (project_id);

-- ---------------------------------------------------------------------------
-- pgvector readiness (task 2.7) -- INTENTIONALLY NOT CREATED HERE
--
-- Phase 2 will add, in its own migration, after `CREATE EXTENSION vector`:
--
--   CREATE TABLE message_embeddings (
--       message_id  BIGINT NOT NULL REFERENCES messages (id),
--       model       TEXT   NOT NULL,            -- supports >1 embedding model
--       embedding   vector(N) NOT NULL,
--       created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
--       PRIMARY KEY (message_id, model)
--   );
--
-- This keeps embeddings off the hot `messages` table (no breaking ALTER) and
-- means this schema runs fully without the `vector` extension installed.
-- ---------------------------------------------------------------------------
