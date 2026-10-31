-- Read-back for #84 finding #12 (openspec/changes/distill-thread-origin-filter,
-- task 4.2; review M5 layer 3), independent of the hub's thread fold.
--
-- Lists every `general` entry that continues or resolves a thread with an
-- earlier mention in a row whose route does not count as `general` (protected,
-- none, or rewritten without its route: route_at <> generated_at). Selects ids
-- and dates only, never text. Must return no rows.
SELECT g.project_path, g.entry_date, t.id AS thread_id
FROM journal_entries g
CROSS JOIN LATERAL unnest(g.open_question_threads || g.resolved_threads) AS t(id)
WHERE g.status = 'entry'
  AND g.route = 'general'
  AND g.route_at = g.generated_at
  AND EXISTS (
        SELECT 1
        FROM journal_entries p
        WHERE p.project_path = g.project_path
          AND p.entry_date < g.entry_date
          AND p.status = 'entry'
          AND t.id = ANY (p.open_question_threads)
          AND NOT coalesce(p.route = 'general' AND p.route_at = p.generated_at, false))
ORDER BY 1, 2, 3;
