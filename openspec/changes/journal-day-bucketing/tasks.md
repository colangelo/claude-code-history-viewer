# Tasks — journal-day-bucketing

## 1. Schema

- [x] 1.1 `migrations/0006_messages_session_timestamp.sql`: `CREATE INDEX IF NOT EXISTS
      messages_session_timestamp_idx ON messages (session_id, "timestamp")`, with a
      comment naming the two callers (windowed fetch, per-day membership probes).

## 2. Hub — shared fold

- [x] 2.1 `journal.rs`: add `pub(crate) const SESSION_DAYS_CTE: &str` — the `msg_days`
      CTE yielding `(entry_date, project_id, session_id)` distinct rows, with `$1` =
      `day_start_hour` and `$2` = optional inclusive `from` date pushed onto
      `messages."timestamp"`.
- [x] 2.2 Add `pub(crate) fn day_bounds(entry_date, day_start_hour) -> (DateTime<Utc>,
      DateTime<Utc>)` for the half-open day window, used by the provenance check and
      exercised directly by unit tests.

## 3. Hub — pending

- [x] 3.1 Rewrite `journal::pending`'s statement on `SESSION_DAYS_CTE`; keep `as_of`,
      the closed-day bound, ordering and `limit` unchanged.
- [x] 3.2 Add provenance drift to the pending predicate: `j.session_ids IS DISTINCT
      FROM g.session_ids`.
- [x] 3.3 Aggregate `session_ids` with `ORDER BY` so the comparison in 3.2 is against a
      canonical array.

## 4. Hub — write endpoint

- [x] 4.1 Replace the provenance query in `journal::upsert` with the sessions-narrowed
      + `EXISTS` form (design Decision 3), bounded by `day_bounds`.
- [x] 4.2 Bind the sorted/deduped `ids` into the INSERT instead of
      `payload.session_ids`, so stored provenance is canonical.
- [x] 4.3 Keep both error texts distinct (membership vs coverage); update the
      membership text to say the session has no message in the day.

## 5. Hub — health

- [x] 5.1 Rewrite `health::healthz_journal`'s `sess` CTE on `SESSION_DAYS_CTE`, keeping
      the `within_days` window and the `arrivals` join.
- [x] 5.2 Add the same provenance-drift condition as 3.2, so health and pending agree.

## 6. Hub — message window

- [x] 6.1 `browse.rs`: add `from` / `to` to the messages `PageParams`, parsed with the
      same helper `/v1/search` uses; 400 on malformed input.
- [x] 6.2 Apply the window to both the page query and the `X-Total-Count` count query.

## 7. Distiller

- [x] 7.1 `Hub.session_messages(session_id, from=None, to=None)` — pass the window
      through as query params.
- [x] 7.2 `build_transcript(hub, session_ids, entry_date)` — compute the day window
      from `entry_date` + the fold hour and fetch only that slice.
- [x] 7.3 `process_group`: when the built transcript is empty, POST a `skip` row and
      make no LLM call.
- [x] 7.4 Keep the fold hour in one place in the script (module constant) and assert it
      matches the hub's in a test.

## 8. Tests

- [x] 8.1 `day_bounds` unit tests: fold at the boundary hour, and the half-open edges.
- [x] 8.2 Hub integration: a session with messages on two logical days appears in both
      pending groups, and in neither for an idle day inside its span.
- [x] 8.3 Hub integration: POST accepted for both days of a spanning session; rejected
      for a session with no message in the posted day; rejected on partial coverage.
- [x] 8.4 Hub integration: provenance drift makes a group pending, and re-POSTing the
      correct set clears it.
- [x] 8.5 Hub integration: `/v1/sessions/:id/messages?from=&to=` returns only the
      window and a windowed `X-Total-Count`; malformed bound is 400.
- [x] 8.6 Health: the same two-day session yields both groups, matching pending.
- [x] 8.7 Distiller: `build_transcript` requests the day window; an empty transcript
      posts `skip` without an LLM call.

## 9. Verification

- [x] 9.1 `cargo test -p hub -- --test-threads=1`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all -- --check`.
- [x] 9.2 Query shapes measured directly against the live archive before writing them
      (numbers in design.md). Verified on the real data that the new grouping produces
      a 2026-08-20 group carrying both spanning sessions (it had none), and grows
      2026-08-19 from 7 stored sessions to 9.
- [x] 9.2b Post-deploy (2026-08-21 00:29Z): `messages_session_timestamp_idx` present on
      pg1, `indisvalid = t`, 215 MB.
- [ ] 9.2c Still open: confirm the windowed fetch *plans* onto the new index, and
      re-measure the pending query now that it exists (design.md's numbers predate it).
- [x] 9.3a Post-deploy API verification (2026-08-21 00:28Z, **on m4m** — not an
      independent cross-machine check): windowed read 200 with
      `X-Total-Count` 64402 vs 67889 unwindowed on session 1231594; malformed bound
      400; `/v1/healthz` ok. `pending` lists 2026-08-19 `infra` with the corrected
      9 sessions (7 stored).
- [x] 9.3b **Done** (infra, one forward tick kickstarted 2026-08-21 ~00:4xZ — bare
      `cchv-distill`, no `--backfill`): the 2026-08-19 `infra` entry re-distilled from
      7 to **9** sessions, and the 19th's feed grew from 4 project entries to **10**.
      Both named sessions are in the set
      `[1195339,1199483,1228471,1228541,1228616,1228726,1231594,1233018,1233156]`.
- [ ] 9.3c Still not checkable, and correctly reported as such rather than as
      "missing": 2026-08-20 is an OPEN logical day until 04:00Z, so `pending` lists no
      08-20 group by design. **Proven from the other direction meanwhile** — through
      the new windowed endpoint, sessions 1231594 and 1233018 each return messages in
      BOTH the 19th's and the 20th's day windows, while control 1233156 returns
      messages on the 19th and **0** on the 20th. That is the midnight-spanning attach
      working; what remains is only the entry itself, after 04:00Z.
- [ ] 9.3d `/v1/healthz/journal` still 503 and legitimately so: the re-grouping left
      64 groups undrained, the one forward tick took its 50-group batch, 45 pending at
      last report and still draining. `status:"stale"` **with a groups list** is the
      tell that it is a backlog, not a stall. Live alongside it: `healthz` 200,
      `ingest?exclude=ac-mbp` 200, `stats` 200.
      ⚠ 6 of those groups (dated 2026-08-13) were **not drainable by any forward
      tick** — see §11.

## 10. Rollout

- [x] 10.1 Released `cchv-v0.19.0` (`91df7b8e`), relayed `576c10a3`, swapped by infra
      and verified live at 00:27:31Z. Follow-ups `e4615da0` (the expected 503) and
      `6cba8c6e` (corrected clear-by time).
- [ ] 10.2 After the first forward tick drains, run the bounded historical repair:
      `cchv-distill --backfill --from 2026-07-04` — 459 groups (287 never distilled,
      172 drifted), 20 per tick by default. Operator's call, not automatic.
- [ ] 10.3 Close `ac/claude-code-history-viewer#35` with the measured before/after —
      **after 9.3c**, which is the before/after worth quoting.

## 11. Horizon anchor — the off-by-one the deploy found

Reported by infra 2026-08-21 (thread `576c10a3`): the forward tick computed
`from=2026-08-14` while `/v1/healthz/journal?within_days=7` — the URL Gatus actually
polls — counted 6 groups dated **2026-08-13**. Neither number is wrong on its own; the
two windows simply disagreed by a day, and 6 groups sat inside the check's window and
outside the tick's, counted stale with nothing able to drain them. Not a rollback
trigger, and not cosmetic either: those groups age out undistilled.

The disagreement is the anchor, not the width. The hub folds by `DAY_START_HOUR` before
subtracting (`health.rs::horizon_from`), so at 00:46Z its "today" was still 08-20; the
distiller used `date.today()`, the machine's **local calendar** date, so its "today" was
08-21. Two defects in one expression — the fold, and local-vs-UTC.

- [x] 11.1 Fixed on the distiller side, which is the half that deviated from the spec
      (`journal-health` already says "of the current logical day"):
      `scripts/cchv-distill.py::journal_today()` folds by `DAY_START_HOUR` in UTC, and
      the forward `from` is measured from it.
- [x] 11.2 Hub side: extracted `health.rs::horizon_from(now, day_start_hour,
      within_days)` — same expression, now a named function with unit tests over the
      00:46Z / 03:59:59Z / 04:00Z boundary instead of an inline expression in a handler
      no test could reach.
- [x] 11.3 Pinned across the language boundary, the way `DAY_START_HOUR` already is:
      `test_forward_horizon_anchor_matches_the_hub` reads `horizon_from`'s body out of
      `health.rs` and fails if the anchor drifts, naming the consequence.
- [x] 11.4 Spec delta: the `Distiller job` requirement now states the anchor and carries
      a scenario pinning the 00:46Z case.
- [ ] 11.5 Deploy: **distiller-only for the fix that matters** (`scripts/cchv-distill.py`
      on m4m, no hub swap needed — `horizon_from` is a refactor, byte-equivalent in
      behaviour). Until it lands, the nightly 00:00–04:00Z window keeps producing the
      same false stale. The 6 groups dated 2026-08-13 have almost certainly aged out of
      both windows by now and need the §10.2 backfill, not a tick.
- [ ] 11.6 No Gatus change is owed. `within_days=7` was never the wrong parameter — a
      config-side fix (`within_days=6`) would have been correct for four hours a day and
      wrong for the other twenty.
