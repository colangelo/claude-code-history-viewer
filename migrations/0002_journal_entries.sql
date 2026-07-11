-- journal-entries :: distilled per-(date, project) journal entries (issue #12)
--
-- Additive migration: creates one new table + its indexes. No existing table is
-- touched, so this applies cleanly to a live archive database and rollback is
-- simply "deploy the previous binary" (the unused table is inert).
--
-- A journal entry is the distillation of one *logical date* of activity for one
-- *project path*, folded ACROSS machines: recall is "what happened in project X
-- on date D", not "…on machine M". Machine provenance stays reachable through
-- the entry's `session_ids` (hub surrogate ids into `sessions`).
--
-- `status` discriminates two row kinds sharing the same (entry_date,
-- project_path) key:
--   * 'entry' — a real distillation with headline/summary/topics/open_questions.
--   * 'skip'  — a watermark for a group the distiller judged non-substantive:
--               it carries only the group key + session ids + generated_at, so
--               the pending work list won't re-offer the group until NEW session
--               data arrives, but it never surfaces in browse or search.

CREATE TABLE journal_entries (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    entry_date      DATE        NOT NULL,
    project_path    TEXT        NOT NULL,
    status          TEXT        NOT NULL CHECK (status IN ('entry', 'skip')),
    headline        TEXT,                   -- one line (entry only)
    summary         TEXT,                   -- 2–5 sentences (entry only)
    topics          TEXT[]      NOT NULL DEFAULT '{}',   -- 3–8 for entries
    open_questions  TEXT[]      NOT NULL DEFAULT '{}',   -- dropped/unresolved threads
    -- Hub surrogate session ids this entry distills (references sessions.id;
    -- kept as a plain array, not an FK, so an accidental session delete can't
    -- cascade away a journal entry — same cumulative-archive stance as messages).
    session_ids     BIGINT[]    NOT NULL DEFAULT '{}',
    model           TEXT,                   -- model that generated the entry
    generated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Flattened plaintext (headline + summary + topics + open_questions), built
    -- by the write handler. NULL for skip rows so they never match FTS.
    search_text     TEXT,
    text_search     TSVECTOR
        GENERATED ALWAYS AS (to_tsvector('simple', coalesce(search_text, ''))) STORED,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One entry per logical date per project, folded across machines.
    UNIQUE (entry_date, project_path)
);

-- Full-text search over entry text (skip rows have a NULL search_text → empty
-- tsvector → never match).
CREATE INDEX journal_entries_text_search_idx ON journal_entries USING GIN (text_search);
-- Browse "a project's journal, newest-first" and the pending join by group key.
CREATE INDEX journal_entries_project_date_idx ON journal_entries (project_path, entry_date DESC);
