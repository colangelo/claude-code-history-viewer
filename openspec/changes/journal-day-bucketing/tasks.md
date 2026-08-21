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

> **Everything in this section is done in the repo and NOT RUNNING in production.**
> `~/.local/bin/cchv-distill` is a **copy**, dated Jul 24, and the reinstall is §11.5.
> The `[x]` below mean "written and tested", which is the only thing a checkbox in a
> repo can honestly mean about a file that gets *copied* somewhere else to run. See
> §12 for what that costs — it is not only the horizon anchor.

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
- [ ] 9.2c Still open (tracked as **#36**, with numbers): confirm the windowed fetch *plans* onto the new index, and
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
- [x] 9.3c **DONE, verified 2026-08-21 07:47Z: the 2026-08-20 entries exist** — 20 of
      them, `/Users/ac/_sync/dev/infra` carrying 15 sessions, a day that had none at
      all before. Sessions 1231594 and 1233018 are under BOTH the 19th and the 20th,
      and the 20/8 headline names the vik launch the 19/8 entry used to claim.
      That is the reported bug, structurally fixed. Superseded text below:
      ~~Still not checkable, and correctly reported as such rather than as~~
      "missing": 2026-08-20 is an OPEN logical day until 04:00Z, so `pending` lists no
      08-20 group by design. **Proven from the other direction meanwhile** — through
      the new windowed endpoint, sessions 1231594 and 1233018 each return messages in
      BOTH the 19th's and the 20th's day windows, while control 1233156 returns
      messages on the 19th and **0** on the 20th. That is the midnight-spanning attach
      working; what remains is only the entry itself, after 04:00Z.
- [x] 9.3d **DONE: green at 07:44Z and again at 09:5xZ** — `status:"ok"`, 10 groups,
      0 stale. The backlog drained as predicted; no intervention was needed.
      ⚠ **But it now answers in 10.0 s** (measured 09:5xZ), worse than the 6.9 s that
      produced the 01:29Z 502 — the live hub still runs the two-pass query, and the
      single-pass fix (3.7 s, `c31d31bd`) is on `main` awaiting a swap. This is not
      "bundle it whenever": at 10 s it is past most monitor timeouts and will flap
      again. Infra reproduced the pre-swap cost independently: `within_days=7`
      measured **6.73 s** at 08:19Z on the deployed binary (200, 2 groups, 0 stale).
      **`b073deba` was the wrong ref** (infra, 2026-08-21) — it touches this file and
      no `.rs` at all. `c31d31bd` is *both* the single-pass health query and the
      `clippy::doc_markdown` backtick fix for the un-backticked `DarkWake` that
      `b448d83c` introduced into `health.rs`. So they are one commit, not two: a
      cherry-pick or a bisect that took `b073deba` for either would get neither.
      Superseded text below:
      ~~`/v1/healthz/journal` still 503 and legitimately so:~~ the re-grouping left
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
- [x] 10.3 **#35 closed** on the prose diff: the vik board moved from the 2026-08-19
      entry to 2026-08-20 where it happened, and the 08-19 entry greps clean for every
      08-20-only subject. Both entries generated after the 08:15Z distiller reinstall,
      so both came from windowed transcripts.

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
- [x] 11.5 **DONE — reinstalled by infra 2026-08-21, verified on run 206.** Deploy was
      **distiller-only for the fix that matters** (`scripts/cchv-distill.py` on m4m, no
      hub swap needed — `horizon_from` is a refactor, byte-equivalent in behaviour).
      The 6 groups dated 2026-08-13 have aged out of both windows and need the §10.2
      backfill, not a tick.

      Landed as `install -m 755` from main `08604fbd`, blob `c1d4a4ce`, extracted with
      `git cat-file blob` rather than copied from the worktree — that tree is
      Syncthing-shared, so a peer write mid-copy would not have shown up. Installed copy
      `cmp`-identical to the blob. The Jul-24 predecessor (`b69f78a4…`) is kept at
      `~/.config/cchv/staging/cchv-distill.preswap-2026-08-21` as the rollback point.
      The plist diff was comment-only (every functional key byte-identical), so infra
      deliberately did **not** reload — `bootstrap` fires `RunAtLoad`, and an
      unrequested prod journal write is ac's call.

      **Run 206, 08:45:52Z**: `backend=aiproxy model=gpt-5.6-sol effort=low`, then
      `27 group(s) pending (from=2026-08-14, limit=50)`, straight into distilling.
      `from=2026-08-14` is `journal_today()` and matches healthz — though *not* the
      discriminating case, since at 08:45Z `date.today()` would also have said 08-14.
      The discriminating window is 00:00–04:00Z, which is what `ac/infra#116` stays open
      on. Same run is n=164 for the exit-timer rule: it opened at exactly run 205's
      `done:` + 3600 s.

      What was true until then, and is the transferable half: `~/.local/bin/cchv-distill`
      line 588 was still `(date.today() - timedelta(days=args.horizon_days)).isoformat()`
      with neither `journal_today` nor `DAY_START_HOUR` anywhere in the file — an
      installed script is a *copy*, so nothing about a green `main` says anything about
      what is running.
- [x] 11.6 Confirmed by the outcome — no Gatus *condition* change was made or needed.
      (Infra did later disable the ntfy **page** on that one endpoint as a bounded,
      tracked suppression under `ac/infra#118` while #37 was open; the check itself was
      never changed, and re-enabling is part of that issue.) Original: No Gatus change is owed. `within_days=7` was never the wrong parameter — a
      config-side fix (`within_days=6`) would have been correct for four hours a day and
      wrong for the other twenty.

## 12. The reinstall carries #35's content fix, not just the horizon anchor

Found 2026-08-21 07:50Z, after §11.5 had already established that
`~/.local/bin/cchv-distill` is a stale Jul-24 copy. §11.5 reads the consequence as the
nightly false stale, and calls the reinstall "distiller-only for the fix that matters".
That undersells it, and the undersell is the dangerous part: it invites the reinstall
to be scheduled as a monitoring nicety.

**The stale copy also means task 7.2 has never run.** Verified in the installed file,
not inferred: `build_transcript(hub, session_ids)` takes no `entry_date` and
`session_messages(self, session_id)` takes no window, so every group is still
summarised from **whole sessions** — and `truncate()` keeps the tail 40%, which is the
newest content. That is the exact mechanism of #35.

So the fix has landed in two halves and only one of them is live:

| half | what it fixes | state |
|---|---|---|
| hub grouping (§2–§5) | *which* sessions belong to a day; the missing next-day entries | **live** since `cchv-v0.19.0`, verified |
| distiller windowing (§7) | *which messages* reach a day's prompt — the wrong-day text the user reported | ~~not deployed~~ → **live** since the §11.5 reinstall, 2026-08-21 |

**The §7 half needed no hub swap, and we did not claim that** (infra, 2026-08-21).
`from`/`to` on `GET /v1/sessions/{id}/messages` shipped in `d7d561be` — the *same*
commit as the grouping fix, and an ancestor of `cchv-v0.19.0` — so the hub end of the
windowing has been live since that release and only the installed script was stale.
Measured against the deployed hub on session 1231594: unwindowed `X-Total-Count`
**67889**, `[08-19T04:00Z, 08-20T04:00Z)` → **3487**, `[08-20T04:00Z, 08-21T04:00Z)` →
**64402**, and 3487 + 64402 = 67889 exactly, so the two logical-day windows partition
the session with no overlap and no gap. The reinstall therefore closed #35's second
half on its own. Worth generalising: *"which halves are deployed"* is a question about
artifacts, not about sections — one commit here spanned both, and reading the section
numbering as the deploy boundary understated what was already live.

Consequence for anything already verified: every journal entry generated so far —
including the 2026-08-20 entries confirmed at 07:47Z, and the re-distilled 2026-08-19
`infra` entry — was produced by the **old whole-session** distiller. Their
`session_ids` are correct because the hub computed them. Their **prose is not
established** to be free of neighbouring-day content, and any read of it that says
otherwise is reading LLM variance.

- [x] 12.1 Re-stated to infra (relay `484d61a9`, thread `576c10a3`). Re-state the ask to infra so the reinstall is not scheduled as a monitoring
      nicety: it is the deployment of #35's content fix. (§11.5 stays the mechanism.)
- [x] 12.2 **DONE** — both days were re-distilled by the 08:49Z and 09:52Z ticks, i.e.
      after the 08:15Z reinstall, so both used windowed transcripts. The prose diff is
      on #35: the vik board moved from the 19th to the 20th, and the 19th greps clean
      for every 20th-only subject. Superseded: After the reinstall, re-distil 2026-08-19 and 2026-08-20 for
      `/Users/ac/_sync/dev/infra` and diff the prose against today's. That, not the
      session-id count, is the before/after #35 should be closed on.

      ⚠ **This plan has no lever, and that is by design** (infra, 2026-08-21).
      `journal.rs::pending` makes a group pending on three *data-derived* conditions —
      no row, `session_ids` drift, `ingest_xid` not visible in `generated_snapshot` —
      and a distiller-side windowing change moves none of them. Our own docstring is
      the reason: dirtiness comes "from the data, not from an operator remembering to
      pass a flag". So a re-distil is a **side effect of unrelated ingest**, not
      something this change can cause. Do not plan a before/after that assumes it can.

      Where the two days actually stand (infra measured both, and retracted a first
      reading that said neither could fire — 28 minutes falsified it):
      - **2026-08-19 `/Users/ac/_sync/dev/infra`** — old entry `generated_at`
        `2026-08-21T04:50:43Z` (run 203); not pending at 08:19Z, **pending at 08:47Z**
        after ongoing ingest dirtied it (the work list went 2 → 27 groups in that
        window). It re-distils on a forthcoming tick, free.
      - **2026-08-20 `/Users/ac/_sync/dev/infra`** — old entry `07:41:03Z` (run 205);
        still not pending. Deleting its `journal_entries` row is the only lever the
        schema offers, and that is an operator action, not a code path.

      The "before" prose is **perishable and was captured first** — both entries in
      full at `~/.config/cchv/staging/journal-prose-before-windowing-2026-08-21.json`
      **on m4m** (infra's filesystem, not ours). The 19th going pending makes that
      load-bearing rather than precautionary: it is queued to be overwritten.
      Suggestive already, though the diff is ours to do: the **19th's** topics include
      "Vikunja project board", "measurement and timezone corrections" and "handoff
      verification" — which is the 20th–21st's work.
- [ ] 12.3 Make the copy self-evident: either install by symlink, or have the distiller
      log its own version/commit at tick start so a stale copy is visible in the log it
      already writes. Today nothing running says which build it is.

**The transferable half.** §11.5 already wrote the rule — *"an installed script is a
copy, so nothing about a green `main` says anything about what is running"* — and then
applied it to one task. The rule is about the **install boundary**, so it applies to
every task on the far side of it at once. When a deploy gap is found, re-check the whole
section it sits in, not the task that surfaced it.

## 13. Dirty detection at the grouping's granularity (#37)

The defect this change shipped, found 2026-08-21 09:34Z on the live archive while
explaining a group count infra could not account for. §2 moved *grouping* from session
to day and left *dirty detection* on `sessions.ingest_xid`, so one still-running session
re-dirtied every day it had ever touched, on every ingest — frozen days re-distilled
forever at ~20 s/group, and `/v1/healthz/journal` permanently 503.

Confirmed from two independent instruments: here, session 1133739's 2026-08-15 messages
last arrived 2026-08-16 while the session was still being written on 2026-08-21; and by
infra, 7 of 9 groups distilled at 08:51Z pending again by 09:36Z (78% re-work), with one
project re-dirtied on four separate frozen days.

- [x] 13.1 `migrations/0008_messages_ingest_xid.sql` — `ADD COLUMN` with **no** default
      (metadata-only; a volatile default rewrites 7.3 M rows and holds ACCESS EXCLUSIVE),
      then `SET DEFAULT pg_current_xact_id()` for new rows. Existing rows stay NULL and
      read as visible, so the migration does not mark the archive dirty. Numbered 0008,
      not 0007 — the tick-observability work took 0007 first, and two files at one
      version is a duplicate-version error, not a merge.
- [x] 13.2 `journal::pending` — the ingest test now asks the group's **own day's**
      messages (`m.session_id = ANY(g.session_ids)` bounded by the day window, using
      `messages_session_timestamp_idx`). `grp` no longer aggregates session xids.
- [x] 13.3 `health::healthz_journal` — same change, so the two still agree.
- [x] 13.4 Regression test `a_live_session_does_not_dirty_its_own_frozen_day`. Verified
      meaningful: reverted to the session-granular form it fails with **both** days
      pending; on the fix only the day that gained a message is.
- [x] 13.5 Spec delta: the pending requirement now states the day scoping and carries a
      scenario for it.
- [x] 13.6 Released `cchv-v0.20.1` (`ac3945ec`), swapped by infra 2026-08-21. Migration
      0008 executed in **13 ms** (metadata-only, against 0006's 6.66 s with its index
      build). `healthz/journal` 503/3 groups → **200/ok/0 groups**, and it cleared **at
      the migration, not at a tick** — NULL-reads-visible empties the dirty set the
      moment the column exists. Verified under live load: 2,983 messages ingested for
      cchv in ten minutes with `pending` empty, i.e. the session that caused the bug
      proved it fixed while writing.
      ORIGINAL TEXT: Release + deploy. Infra is holding for it and has silenced the `cchv-journal`
      **page** (not the check) under `ac/infra#118`, which names this fix as the expiry
      condition. **Closing that issue is part of this deploy, not a follow-up.**

**Two test-harness bugs this uncovered, both of which made a test lie rather than fail:**

- `seed()` keyed messages by array index, so a *second* seed of the same session
  restarted at 0, collided, upserted to a no-op — and the test observed "no new data"
  while believing it had ingested some. Keys are timestamp-derived now.
- `pending_for()` read absence off a possibly-truncated page (fixed earlier in §8).

Both have the same shape as the product bugs this change has been chasing all day: a
check that cannot fail, reporting success. Worth noticing that the test harness is not
exempt from the class of bug the tests exist to catch.
