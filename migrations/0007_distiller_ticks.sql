-- Distiller tick records (change: distiller-tick-observability).
--
-- An idle distiller tick is invisible to the hub. Its only hub interaction is
-- `GET /v1/journal/pending`, and a GET writes nothing — so a distiller ticking
-- hourly into an empty work list and one that has not run in four hours leave
-- exactly the same trace: none. `/v1/healthz/journal` therefore returned the
-- same 503 for "a backlog is draining" and "nothing is draining it", and two
-- separate derivations used the hourly `StartInterval` to predict a clear-by
-- time that no tick was coming to deliver (measured 2026-08-21: 3 h 45 m,
-- `state = not running`, because launchd does not fire intervals while the host
-- sleeps).
--
-- Deliberately not derived from `journal_entries.generated_at`: that timestamp
-- moves only when a tick both had work AND succeeded at it, which reproduces the
-- same ambiguity one level down.
--
-- A log rather than a single upserted row, so the endpoint can also answer
-- "how many ticks in the last 24h" — the number that makes a wake-driven cadence
-- visible instead of inferred. At ~24 rows/day, pruned to 30 days, this table
-- holds hundreds of rows.
CREATE TABLE IF NOT EXISTS distiller_ticks (
    id             bigserial   PRIMARY KEY,
    -- Server-assigned: the distiller's clock is not the archive's, and the
    -- health check compares this against the database's `now()`.
    tick_at        timestamptz NOT NULL DEFAULT now(),
    -- 'forward' (the scheduled job) or 'backfill' (an operator's bounded
    -- historical run). Both drain real work and both count as ticks; the mode is
    -- what lets a reader tell a scheduled tick from a hand-run one.
    mode           text        NOT NULL,
    -- Groups the tick found pending when it started. Recorded before any LLM
    -- call, so this is the size of the work list, not of the work completed.
    groups_pending integer     NOT NULL
);

-- Every read of this table is "the most recent tick" or "ticks since T".
CREATE INDEX IF NOT EXISTS distiller_ticks_tick_at_idx
    ON distiller_ticks (tick_at DESC);
