-- The route each journal entry was generated on (#84 finding #12,
-- openspec/changes/distill-thread-origin-filter).
--
-- `route` is `general` or `protected`, posted by the distiller; null means "not
-- known", which the thread fold treats as protected origin. `route_at` is
-- written in the same statement as `generated_at`, so a row rewritten by an
-- older hub (which does not know these columns) no longer matches and its route
-- stops counting (review M6).
--
-- No backfill: every existing row keeps a null route (review M1, decided by the
-- client's manager). A later backfill needs ac's word.
ALTER TABLE journal_entries
    ADD COLUMN IF NOT EXISTS route    TEXT,
    ADD COLUMN IF NOT EXISTS route_at TIMESTAMPTZ;

ALTER TABLE journal_entries DROP CONSTRAINT IF EXISTS journal_entries_route_check;
ALTER TABLE journal_entries
    ADD CONSTRAINT journal_entries_route_check CHECK (route IN ('general', 'protected'));
