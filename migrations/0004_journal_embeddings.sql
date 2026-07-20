-- journal_embeddings :: derived sentence embeddings for semantic journal search
--
-- Additive migration: one new side table, no existing table touched, no
-- extension required (plain REAL[] — deliberately NOT pgvector at journal
-- scale; the message-scale phase graduates to pgvector via a NEW additive
-- migration). Rollback is "deploy the previous binary": the table sits inert.
--
-- Rows are DERIVED DATA keyed by (journal entry, model): the hub's background
-- sweep (re)generates them from journal_entries content, so deleting any or
-- all rows is always safe. `content_hash` is the dirty marker — the sweep
-- re-embeds when the entry's embedded text no longer hashes to it. `model`
-- scopes the cosine space: vectors from different models are never compared,
-- and rows from a retired model are simply superseded by the active model's
-- sweep (old rows are harmless and CASCADE away with their entry).
--
-- FK note: journal_entries rows are upserted in place by (entry_date,
-- project_path) — the surrogate id is stable across regenerations, so the
-- CASCADE only fires if a journal row is truly deleted (which the archive
-- never does in normal operation).

CREATE TABLE journal_embeddings (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    journal_entry_id  BIGINT      NOT NULL
        REFERENCES journal_entries (id) ON DELETE CASCADE,
    model             TEXT        NOT NULL,
    dim               SMALLINT    NOT NULL,
    embedding         REAL[]      NOT NULL,
    content_hash      TEXT        NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One vector per (entry, model); the sweep upserts on this key.
    UNIQUE (journal_entry_id, model)
);

-- Semantic queries load the active model's full (small) vector set.
CREATE INDEX journal_embeddings_model_idx ON journal_embeddings (model);
