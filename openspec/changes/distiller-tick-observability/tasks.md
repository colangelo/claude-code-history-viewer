# Tasks — distiller-tick-observability

> **Archive order.** This change's spec deltas are written on top of
> `journal-day-bucketing`'s, which is still in flight. Archive that one first, or
> the MODIFIED requirements apply against the wrong baseline.

## 1. Schema

- [x] 1.1 `migrations/0007_distiller_ticks.sql`: `distiller_ticks (id, tick_at,
      mode, groups_pending)` + a `tick_at DESC` index, with a comment naming why an
      idle tick is otherwise invisible.

## 2. Hub — tick record endpoint

- [x] 2.1 `journal.rs`: `record_tick` handler (machine-token auth), validating
      `mode ∈ {forward, backfill}` and `groups_pending >= 0`, returning the stored
      timestamp.
- [x] 2.2 Prune rows older than 30 days on insert, in the same statement's wake.
- [x] 2.3 Route `POST /v1/journal/ticks` in `lib.rs`.

## 3. Hub — health

- [x] 3.1 `health.rs`: report `last_tick_at`, `last_tick_age_secs`,
      `last_tick_mode`, `last_tick_groups_pending`, `ticks_last_24h`.
- [x] 3.2 Optional `max_tick_age_secs` param, absent by default; when set and
      exceeded (or no tick ever), status `"no_tick"` + 503, outranking `"stale"`.
- [x] 3.3 Keep the default verdict byte-identical to today's for callers that do
      not pass the new param.

## 4. Distiller

- [x] 4.1 `Hub.post_tick(mode, groups_pending)` using the existing retry wrapper.
- [x] 4.2 Call it in `main()` right after the pending query succeeds, before any
      LLM call; skip under `--dry-run`.
- [x] 4.3 A tick-record failure must not abort the run — the record is
      observability, not work.

## 5. Docs & comments — the cadence claims the measurement refuted

- [x] 5.1 `scripts/dev.cchv.distiller.plist`: staleness is bounded in wakes, not
      hours; launchd coalesces missed intervals into one catch-up.
- [x] 5.2 `docs/archive/deployment.md` §3c: same correction, plus the day-close
      property and what the new fields are for.
- [x] 5.3 `scripts/cchv-distill.py` retry comment: "next tick", not "within the
      hour".
- [x] 5.4 `health.rs` module/handler docs: a day close is never a net drain.

## 6. Tests

- [x] 6.1 Hub integration: a recorded tick surfaces in `/v1/healthz/journal`
      (timestamp, mode, pending count, `ticks_last_24h`).
- [x] 6.2 Hub integration: invalid mode / negative count → 400, no write;
      unauthenticated → rejected.
- [x] 6.3 Hub integration: `max_tick_age_secs` absent → verdict unchanged; set and
      exceeded → `"no_tick"` 503 even with stale groups present; set and satisfied →
      falls through to the stale verdict.
- [x] 6.4 Hub integration: a day that closes with already-past-grace groups is
      stale immediately (the "close is not a recovery event" property).
- [x] 6.5 Unit: `max_tick_age_secs=0` / non-numeric → 400.
- [x] 6.6 Distiller: a normal run posts exactly one tick record; `--dry-run` posts
      none; a failing tick record does not abort the run.

## 7. Verification

- [x] 7.1 `cargo test -p hub -- --test-threads=1`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all -- --check`.
- [x] 7.2 `scripts/test_cchv_distill.py`.

## 8. Rollout

- [ ] 8.1 Release + hub swap **and** `cchv-distill` reinstall on the hub machine —
      one relay carrying both halves (`docs/archive/deployment.md` §2b, §3c). Order
      does not matter: new-distiller/old-hub swallows a **405** with a WARN, and
      new-hub/old-distiller reports `null` and alerts nobody. **Distiller half DONE**
      2026-08-21 (infra took it early rather than waiting for the swap; it also
      carried `journal-day-bucketing` §11.5). Hub half still owed — it needs a
      release past `cchv-v0.19.0`.

      **The warning is a `405`, not a `404`** (infra's correction of their own probe,
      2026-08-21, and the reason it is worth the line): they read `GET
      /v1/journal/ticks` → 404 as "route absent, since a POST-only route would answer
      405" — then run 206 logged `POST /v1/journal/ticks 405`. The control settles it:
      the certainly-absent `/v1/journal/zzznope` answers GET 404 / POST 405, identical
      to `ticks`, so on this router **405 discriminates nothing** about whether a route
      exists. Verdict (old hub, route absent) unchanged; the status code in the log is
      405. Both are 4xx, so neither is retried and distillation is untouched either way.
- [ ] 8.2 Verify live: `last_tick_at` advances after a tick; `ticks_last_24h`
      sits well under 24 — infra measured **15.38 ticks/day** on this host. It is
      *not* the wake count (40–106 sleep cycles/day), but not because DarkWakes
      run no `StartInterval` agent — they do, and that claim was retracted
      2026-08-21. The cap is the interval re-arming at each run's **exit**
      (≈21.7/day awake) plus sleep coalescing missed intervals into one catch-up.
- [x] 8.3 Infra's call whether the `cchv-journal` Gatus check sets
      `max_tick_age_secs`, and to what. **Answered 2026-08-21: `43200` (12 h)**,
      from a 13-day replay — median gap 68 m, p90 2 h 41 m, max 7 h 41 m (values
      corrected the same day, when the replay was re-run on tick *starts* rather
      than completions); 8 h is the tightest zero-flap value and has no headroom,
      our ~3 h 30 m would have fired 16×. Tracked `ac/infra#117`, and it goes on the URL only after 8.1
      and 8.2. Table + caveats: `docs/archive/deployment.md` §3c. Related: the
      monitoring decision on `ac/infra#94`.
