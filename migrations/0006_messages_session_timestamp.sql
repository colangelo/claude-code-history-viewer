-- Composite index for per-session time windows (change: journal-day-bucketing).
--
-- Two callers want exactly this shape, and both were paying for its absence:
--
--   * GET /v1/sessions/:id/messages?from=&to= — the distiller now fetches one
--     logical day of a session rather than the whole session. Without this index
--     the page plans as a BitmapAnd of messages_session_id_message_key_key and
--     messages_timestamp_idx: 52.6 ms per 500-row page, measured on the live
--     archive (7.2 M messages). The distiller pages through every group, so this
--     is the hot path.
--
--   * The journal provenance check, which asks "does session S have any message
--     inside day D" once per candidate session per POST. On the session-id-only
--     index that is a scan-and-filter over the session's whole key range.
--
-- Additive; no table rewrite. IF NOT EXISTS so a hand-created index (or a
-- re-run) is a no-op rather than a failed migration.
CREATE INDEX IF NOT EXISTS messages_session_timestamp_idx
    ON messages (session_id, "timestamp");
