-- Composite index for /v1/healthz/ingest's per-machine liveness probe (#45).
--
-- The endpoint asks each machine for max(messages.created_at). As one GROUP BY
-- over all of `messages` it is a full sequential scan: 9.4-9.5 s on pg1
-- 2026-09-23 (EXPLAIN: 1,327,705 heap pages read, 3 result rows). That lands
-- just under Gatus's 10 s client timeout, so cchv-ingest-m4m and
-- cchv-ingest-mbm5 read as down while the hub answered a correct 200. The heap
-- stays large after the #30 cleanup (11 GB, ~5.9 GB live), so compaction would
-- only halve it.
--
-- With this index the handler probes each machine with one backward index scan
-- (see health.rs), so the cost follows the number of machines, not rows.
--
-- Same shape as 0006: additive, no table rewrite, IF NOT EXISTS so a
-- hand-created index is a no-op. A plain CREATE INDEX holds off writes to
-- `messages` for the build (one heap pass at hub startup); daemons retry
-- ingest, so it costs a short delay, not data.
CREATE INDEX IF NOT EXISTS messages_machine_created_idx
    ON messages (machine_id, created_at);
