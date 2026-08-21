-- Per-message ingest transaction id (change: journal-dirty-granularity, #37).
--
-- `journal::pending` decides a group is dirty when an ingest committed after the
-- entry's snapshot was taken. It asked that of `sessions.ingest_xid`, which was
-- the right granularity while a session belonged to exactly one group. Since
-- cchv-v0.19.0 a session belongs to every logical day it has messages on, so one
-- long-lived session re-dirtied every day it had EVER touched on every ingest —
-- frozen days re-distilled forever and `/v1/healthz/journal` permanently 503.
--
-- Measured on the live archive 2026-08-21: session 1133739's 2026-08-15 messages
-- last arrived 2026-08-16, the session was still being written on 2026-08-21,
-- and it dirtied the 2026-08-15 group indefinitely. All 19 pending groups had
-- been distilled minutes earlier.
--
-- The fix is granularity, not mechanism: keep `pg_visible_in_snapshot`, ask it
-- about the messages of THAT DAY.
--
-- Two statements on purpose, and the order is load-bearing:
--
--   * `ADD COLUMN` with NO default is metadata-only. A volatile default
--     (`pg_current_xact_id()`) in the same statement would rewrite all 7.3 M
--     rows and hold ACCESS EXCLUSIVE on `messages` for the duration.
--   * `SET DEFAULT` afterwards applies to new rows only.
--
-- So existing rows stay NULL, which the dirty test reads as "ingested before
-- this column existed" and therefore visible. That is what stops the migration
-- from marking the whole archive dirty and triggering a ~1,300-group
-- re-distillation on deploy. It also means the fix is prospective: a group whose
-- messages all predate this migration can no longer be dirtied by ingest at all,
-- which is correct — their content is frozen.
--
-- New rows are stamped by the same transaction that stamps `sessions.ingest_xid`
-- (the ingest INSERT does not name this column, so the default fires inside that
-- transaction), so the two agree by construction rather than by convention.
ALTER TABLE messages ADD COLUMN ingest_xid XID8;
ALTER TABLE messages ALTER COLUMN ingest_xid SET DEFAULT pg_current_xact_id();

COMMENT ON COLUMN messages.ingest_xid IS
    'Transaction that inserted this row; NULL for rows predating migration 0007 '
    '(treated as visible/not-dirty). Day-scoped counterpart to sessions.ingest_xid '
    'for journal pending detection — see #37.';
