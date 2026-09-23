-- Second pass after scripts/dedup-synthesized-rows.sql (cchv #30): leave ONE
-- row per uuid-less record, carrying the key the fixed daemon computes.
--
-- WHY: the first pass kept the LOWEST id in each group, which carries the
-- pre-b4321cf4 key (it hashed a parse-time timestamp). The fixed daemon looks
-- for its own key, doesn't find it, and inserts one more copy. So each record
-- ended at two rows and the duplicate count could never reach 0 (found by
-- infra, 2026-09-23 16:11Z: 3,358 two-row groups, each an old-key keeper plus
-- a post-delete insert).
--
-- THE KEY the daemon computes for a record whose uuid it synthesized, when
-- content, toolUse and toolUseResult are all null:
--   sha256(provider 0x00 sessions.session_id 0x00 0x00 type 0x00 seq-as-i32-LE)
-- Verified on pg1 2026-09-23 against rows the fixed daemons wrote: 2,420 of
-- 2,420 matched, and 0 of 5,000 pre-fix rows did (so the match is not trivial).
-- All 176,265 synthesized-uuid rows had null payloads at that point.
--
-- WHAT IT DOES, per group (session, type, seq, raw minus uuid/timestamp):
--   keeper = the row already holding the new key if there is one, else the
--            lowest id;
--   the rest are deleted (batched, with the same reference guard);
--   a keeper still on an old key is re-keyed to the new one, unless another
--   row already holds that key (then it is left alone; see the last query).
--
-- Run as the hub's role: psql "$URL" -v ON_ERROR_STOP=1 -f scripts/rekey-synthesized-rows.sql
-- Safe to re-run. Concurrent ingest is fine: a daemon insert racing the
-- re-key hits the unique index and is ignored, or the re-key of that row is
-- skipped. Either way it ends with one row per record.

\set ON_ERROR_STOP 1
\timing on

CREATE OR REPLACE FUNCTION cchv_daemon_key(provider text, sid text, type text, seq int)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT encode(sha256(
        convert_to(provider, 'UTF8') || '\x00'::bytea ||
        convert_to(sid, 'UTF8')      || '\x00'::bytea ||
                                        '\x00'::bytea ||   -- the omitted timestamp
        convert_to(type, 'UTF8')     || '\x00'::bytea ||
        substring(int4send(seq) FROM 4 FOR 1) || substring(int4send(seq) FROM 3 FOR 1) ||
        substring(int4send(seq) FROM 2 FOR 1) || substring(int4send(seq) FROM 1 FOR 1)
    ), 'hex')
$$;

-- 1. Classify every synthesized-uuid row with a null payload.
DROP TABLE IF EXISTS cchv_rekey;
SET work_mem = '256MB';
CREATE UNLOGGED TABLE cchv_rekey AS
WITH c AS (
    SELECT m.id, m.session_id, m.message_key,
           cchv_daemon_key(m.provider, s.session_id, m.type, m.seq) AS nk,
           row_number() OVER (
               PARTITION BY m.session_id, m.type, m.seq, (m.raw - 'uuid' - 'timestamp')
               ORDER BY (m.message_key = cchv_daemon_key(m.provider, s.session_id, m.type, m.seq)) DESC, m.id
           ) AS rn
    FROM messages m
    JOIN sessions s ON s.id = m.session_id
    WHERE m.uuid LIKE '%-line-%'
      AND m.raw->'content' = 'null'::jsonb
      AND m.raw->'toolUse' = 'null'::jsonb
      AND m.raw->'toolUseResult' = 'null'::jsonb
)
SELECT id, session_id, message_key, nk, rn = 1 AS keeper FROM c;
CREATE UNIQUE INDEX ON cchv_rekey (id);
RESET work_mem;

SELECT count(*) FILTER (WHERE keeper) AS keepers,
       count(*) FILTER (WHERE NOT keeper) AS victims,
       count(*) FILTER (WHERE keeper AND message_key <> nk) AS keepers_to_rekey
FROM cchv_rekey;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM cchv_rekey v
        WHERE NOT v.keeper
          AND (EXISTS (SELECT 1 FROM message_tool_uses t WHERE t.message_ref = v.id)
               OR EXISTS (SELECT 1 FROM message_tool_results r WHERE r.message_ref = v.id))
    ) THEN
        RAISE EXCEPTION 'a victim row is referenced by a tool use/result: aborting before any change';
    END IF;
END $$;

-- 2. Delete the non-keepers in batches.
CREATE OR REPLACE PROCEDURE cchv_rekey_delete(batch int DEFAULT 50000)
LANGUAGE plpgsql AS $$
DECLARE lo bigint := 0; hi bigint; n bigint; total bigint := 0;
BEGIN
    LOOP
        SELECT max(id) INTO hi
        FROM (SELECT id FROM cchv_rekey WHERE NOT keeper AND id > lo ORDER BY id LIMIT batch) s;
        EXIT WHEN hi IS NULL;
        DELETE FROM messages m USING cchv_rekey v
        WHERE m.id = v.id AND NOT v.keeper AND v.id > lo AND v.id <= hi;
        GET DIAGNOSTICS n = ROW_COUNT;
        total := total + n;
        RAISE NOTICE 'deleted % (total %) through id %', n, total, hi;
        lo := hi;
        COMMIT;
    END LOOP;
END $$;
CALL cchv_rekey_delete();

-- 3. Re-key the keepers still on an old key. One row per (session, key): two
--    groups that differ only in payload map to the same daemon key, and the
--    unique index allows one. A row-level retry absorbs a daemon insert that
--    races the batch.
CREATE OR REPLACE PROCEDURE cchv_rekey_update(batch int DEFAULT 20000)
LANGUAGE plpgsql AS $$
DECLARE lo bigint := 0; hi bigint; n bigint; total bigint := 0; r record;
BEGIN
    LOOP
        SELECT max(id) INTO hi
        FROM (SELECT id FROM cchv_rekey WHERE keeper AND message_key <> nk AND id > lo ORDER BY id LIMIT batch) s;
        EXIT WHEN hi IS NULL;
        BEGIN
            UPDATE messages m SET message_key = k.nk
            FROM (
                SELECT DISTINCT ON (session_id, nk) id, session_id, nk
                FROM cchv_rekey
                WHERE keeper AND message_key <> nk AND id > lo AND id <= hi
                ORDER BY session_id, nk, id
            ) k
            WHERE m.id = k.id
              AND NOT EXISTS (SELECT 1 FROM messages x
                              WHERE x.session_id = k.session_id AND x.message_key = k.nk);
            GET DIAGNOSTICS n = ROW_COUNT;
        EXCEPTION WHEN unique_violation THEN
            n := 0;
            FOR r IN SELECT id, nk FROM cchv_rekey
                     WHERE keeper AND message_key <> nk AND id > lo AND id <= hi LOOP
                BEGIN
                    UPDATE messages SET message_key = r.nk WHERE id = r.id;
                    n := n + 1;
                EXCEPTION WHEN unique_violation THEN NULL;
                END;
            END LOOP;
        END;
        total := total + n;
        RAISE NOTICE 're-keyed % (total %) through id %', n, total, hi;
        lo := hi;
        COMMIT;
    END LOOP;
END $$;
CALL cchv_rekey_update();

-- 4. Postconditions.
--    remaining_duplicate_groups: 0 unless rows arrived during the run (re-run).
SELECT count(*) AS remaining_duplicate_groups
FROM (
    SELECT 1 FROM messages
    WHERE uuid LIKE '%-line-%'
    GROUP BY session_id, type, seq, (raw - 'uuid' - 'timestamp')
    HAVING count(*) > 1
) d;
--    old_key_rows: synthesized rows the daemon would still not recognise. Only
--    the payload-collision case above should remain; the daemon never re-sends
--    those as distinct rows either, because their daemon key is taken.
SELECT count(*) AS old_key_rows
FROM messages m JOIN sessions s ON s.id = m.session_id
WHERE m.uuid LIKE '%-line-%'
  AND m.message_key <> cchv_daemon_key(m.provider, s.session_id, m.type, m.seq);

DROP PROCEDURE cchv_rekey_update(int);
DROP PROCEDURE cchv_rekey_delete(int);
DROP TABLE cchv_rekey;
DROP FUNCTION cchv_daemon_key(text, text, text, int);
