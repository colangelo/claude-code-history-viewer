-- Journal open-question threads (Gitea #15,
-- openspec/changes/journal-open-questions-report, design D2).
--
-- A thread is the set of open questions that carry the same id. The ids live on
-- the entry row, aligned with `open_questions`, so a re-distill replaces a day's
-- links and resolutions together with the rest of the row and nothing can be
-- orphaned. The hub draws new ids from the sequence; the distiller only ever
-- repeats ids it was shown.
CREATE SEQUENCE journal_thread_id_seq;

ALTER TABLE journal_entries
    -- One thread id per open question, same order as `open_questions`.
    ADD COLUMN open_question_threads BIGINT[] NOT NULL DEFAULT '{}',
    -- Threads this day's work settled.
    ADD COLUMN resolved_threads      BIGINT[] NOT NULL DEFAULT '{}';

-- Every question written before this change becomes its own thread, oldest
-- entries first, so the distiller can link to them from its first run.
-- Restatements among them stay separate threads: grouping them by embedding
-- was measured and rejected (design D1).
-- Numbered with row_number() rather than nextval(): nextval inside an ordered
-- aggregate runs in scan order, so "oldest first" would not actually hold.
UPDATE journal_entries e
SET open_question_threads = s.ids
FROM (
    SELECT n.id, array_agg(n.rn ORDER BY n.ord) AS ids
    FROM (
        SELECT e2.id, q.ord,
               row_number() OVER (ORDER BY e2.entry_date, e2.id, q.ord) AS rn
        FROM journal_entries e2
        CROSS JOIN LATERAL generate_subscripts(e2.open_questions, 1) AS q(ord)
    ) n
    GROUP BY n.id
) s
WHERE e.id = s.id;

-- Continue the sequence after the seeded ids (an empty archive starts at 1).
SELECT setval('journal_thread_id_seq', coalesce(max(t), 1), max(t) IS NOT NULL)
FROM journal_entries, unnest(open_question_threads) AS t;

ALTER TABLE journal_entries
    ADD CONSTRAINT journal_entries_thread_links_aligned
    CHECK (cardinality(open_question_threads) = cardinality(open_questions));
