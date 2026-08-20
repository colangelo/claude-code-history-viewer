# Tasks — journal-day-bucketing

## 1. Schema

- [ ] 1.1 `migrations/0006_messages_session_timestamp.sql`: `CREATE INDEX IF NOT EXISTS
      messages_session_timestamp_idx ON messages (session_id, "timestamp")`, with a
      comment naming the two callers (windowed fetch, per-day membership probes).

## 2. Hub — shared fold

- [ ] 2.1 `journal.rs`: add `pub(crate) const SESSION_DAYS_CTE: &str` — the `msg_days`
      CTE yielding `(entry_date, project_id, session_id)` distinct rows, with `$1` =
      `day_start_hour` and `$2` = optional inclusive `from` date pushed onto
      `messages."timestamp"`.
- [ ] 2.2 Add `pub(crate) fn day_bounds(entry_date, day_start_hour) -> (DateTime<Utc>,
      DateTime<Utc>)` for the half-open day window, used by the provenance check and
      exercised directly by unit tests.

## 3. Hub — pending

- [ ] 3.1 Rewrite `journal::pending`'s statement on `SESSION_DAYS_CTE`; keep `as_of`,
      the closed-day bound, ordering and `limit` unchanged.
- [ ] 3.2 Add provenance drift to the pending predicate: `j.session_ids IS DISTINCT
      FROM g.session_ids`.
- [ ] 3.3 Aggregate `session_ids` with `ORDER BY` so the comparison in 3.2 is against a
      canonical array.

## 4. Hub — write endpoint

- [ ] 4.1 Replace the provenance query in `journal::upsert` with the sessions-narrowed
      + `EXISTS` form (design Decision 3), bounded by `day_bounds`.
- [ ] 4.2 Bind the sorted/deduped `ids` into the INSERT instead of
      `payload.session_ids`, so stored provenance is canonical.
- [ ] 4.3 Keep both error texts distinct (membership vs coverage); update the
      membership text to say the session has no message in the day.

## 5. Hub — health

- [ ] 5.1 Rewrite `health::healthz_journal`'s `sess` CTE on `SESSION_DAYS_CTE`, keeping
      the `within_days` window and the `arrivals` join.
- [ ] 5.2 Add the same provenance-drift condition as 3.2, so health and pending agree.

## 6. Hub — message window

- [ ] 6.1 `browse.rs`: add `from` / `to` to the messages `PageParams`, parsed with the
      same helper `/v1/search` uses; 400 on malformed input.
- [ ] 6.2 Apply the window to both the page query and the `X-Total-Count` count query.

## 7. Distiller

- [ ] 7.1 `Hub.session_messages(session_id, from=None, to=None)` — pass the window
      through as query params.
- [ ] 7.2 `build_transcript(hub, session_ids, entry_date)` — compute the day window
      from `entry_date` + the fold hour and fetch only that slice.
- [ ] 7.3 `process_group`: when the built transcript is empty, POST a `skip` row and
      make no LLM call.
- [ ] 7.4 Keep the fold hour in one place in the script (module constant) and assert it
      matches the hub's in a test.

## 8. Tests

- [ ] 8.1 `day_bounds` unit tests: fold at the boundary hour, and the half-open edges.
- [ ] 8.2 Hub integration: a session with messages on two logical days appears in both
      pending groups, and in neither for an idle day inside its span.
- [ ] 8.3 Hub integration: POST accepted for both days of a spanning session; rejected
      for a session with no message in the posted day; rejected on partial coverage.
- [ ] 8.4 Hub integration: provenance drift makes a group pending, and re-POSTing the
      correct set clears it.
- [ ] 8.5 Hub integration: `/v1/sessions/:id/messages?from=&to=` returns only the
      window and a windowed `X-Total-Count`; malformed bound is 400.
- [ ] 8.6 Health: the same two-day session yields both groups, matching pending.
- [ ] 8.7 Distiller: `build_transcript` requests the day window; an empty transcript
      posts `skip` without an LLM call.

## 9. Verification

- [ ] 9.1 `cargo test -p hub -- --test-threads=1`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all -- --check`.
- [ ] 9.2 Against a scratch copy of the archive (or post-deploy on m4m): confirm the
      pending query's plan and runtime are in the envelope design.md records, and that
      the windowed message fetch uses `messages_session_timestamp_idx`.
- [ ] 9.3 Post-deploy: the 2026-08-19 `infra` entry no longer contains 2026-08-20 work,
      and a 2026-08-20 entry exists carrying the spanning sessions.

## 10. Rollout

- [ ] 10.1 Release + deploy via the `cchv-deploy` skill (hub binary swap on m4m — the
      migration runs at hub startup).
- [ ] 10.2 After the first forward tick drains, run the bounded historical repair:
      `cchv-distill --backfill --from 2026-07-04` — 459 groups (287 never distilled,
      172 drifted), 20 per tick by default. Operator's call, not automatic.
- [ ] 10.3 Close `ac/claude-code-history-viewer#35` with the measured before/after.
