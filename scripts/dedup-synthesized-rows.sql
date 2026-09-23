-- Remove re-parse copies of uuid-less records from `messages` (cchv #30).
--
-- WHAT: Claude Code writes its state records (permission-mode, custom-title,
-- mode, bridge-session, agent-color, ...) with no uuid and no timestamp.
-- history-core fills the uuid with `<random v4>-line-N` and the timestamp with
-- `now()`, and until sync-daemon b4321cf4 `message_key` hashed that timestamp.
-- So every re-parse of a growing session file stored every such record again.
-- On 2026-09-23 this was 17,876,920 of 19.3M rows, collapsing to 165,698
-- real records.
--
-- WHICH ROWS: rows whose uuid was synthesized, grouped by (session, type, seq,
-- raw minus uuid and timestamp). Copies differ ONLY in uuid and timestamp
-- (verified field by field). The lowest id in each group is kept; the rest go.
-- A row referenced by message_tool_uses / message_tool_results is never
-- deleted (the dry run found 0; the guard below re-checks).
--
-- HOW TO RUN (operator, off-peak), as the hub's database role:
--   psql "$URL" -v ON_ERROR_STOP=1 -f scripts/dedup-synthesized-rows.sql
-- Then, once it finishes:
--   VACUUM (ANALYZE, VERBOSE) messages;   -- or VACUUM FULL to give disk back
--   hub mirror rebuild                     -- the mirror still holds the copies
--
-- Safe to re-run: step 1 rebuilds the victim list from current state, and
-- step 2 only deletes ids on that list. Stopping it midway loses nothing but
-- time; everything committed so far is only ever a deleted copy.

\set ON_ERROR_STOP 1
\timing on

-- 1. Snapshot the victims. Reads `messages` only; about two minutes on pg1.
DROP TABLE IF EXISTS cchv_dedup_victims;
SET work_mem = '256MB';
CREATE UNLOGGED TABLE cchv_dedup_victims AS
SELECT id
FROM (
    SELECT id,
           row_number() OVER (
               PARTITION BY session_id, type, seq, (raw - 'uuid' - 'timestamp')
               ORDER BY id
           ) AS rn
    FROM messages
    WHERE uuid LIKE '%-line-%'
) ranked
WHERE rn > 1;
CREATE UNIQUE INDEX ON cchv_dedup_victims (id);
RESET work_mem;

SELECT count(*) AS victims FROM cchv_dedup_victims;

-- Guard: stop before deleting anything if a victim is referenced.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM cchv_dedup_victims v
        WHERE EXISTS (SELECT 1 FROM message_tool_uses t WHERE t.message_ref = v.id)
           OR EXISTS (SELECT 1 FROM message_tool_results r WHERE r.message_ref = v.id)
    ) THEN
        RAISE EXCEPTION 'a victim row is referenced by a tool use/result: aborting before any delete';
    END IF;
END $$;

-- 2. Delete in batches, committing each one, so no single transaction holds
--    17M row locks or a huge WAL burst, and ingest keeps flowing throughout.
CREATE OR REPLACE PROCEDURE cchv_dedup_delete(batch int DEFAULT 50000)
LANGUAGE plpgsql AS $$
DECLARE
    lo bigint := 0;
    hi bigint;
    n  bigint;
    total bigint := 0;
BEGIN
    LOOP
        SELECT max(id) INTO hi
        FROM (SELECT id FROM cchv_dedup_victims WHERE id > lo ORDER BY id LIMIT batch) s;
        EXIT WHEN hi IS NULL;
        DELETE FROM messages m
        USING cchv_dedup_victims v
        WHERE m.id = v.id AND v.id > lo AND v.id <= hi;
        GET DIAGNOSTICS n = ROW_COUNT;
        total := total + n;
        RAISE NOTICE 'deleted % (total %) through id %', n, total, hi;
        lo := hi;
        COMMIT;
    END LOOP;
END $$;

CALL cchv_dedup_delete();

-- 3. Postcondition: no synthesized-uuid group has more than one row.
--    Expect 0 when no ingest landed during the run. The fixed daemon (b4321cf4)
--    re-keys each state record ONCE the first time it re-parses a file, which
--    adds one copy per record; any of those that arrived after step 1 show up
--    here. Run the script once more and they go too. After that one pass per
--    file the keys are stable, which is the whole point of the daemon fix.
SELECT count(*) AS remaining_duplicate_groups
FROM (
    SELECT 1 FROM messages
    WHERE uuid LIKE '%-line-%'
    GROUP BY session_id, type, seq, (raw - 'uuid' - 'timestamp')
    HAVING count(*) > 1
) d;

DROP PROCEDURE cchv_dedup_delete(int);
DROP TABLE cchv_dedup_victims;
