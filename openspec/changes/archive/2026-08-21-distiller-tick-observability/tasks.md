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

- [x] 8.1 Release + hub swap **and** `cchv-distill` reinstall on the hub machine —
      one relay carrying both halves (`docs/archive/deployment.md` §2b, §3c). Order
      does not matter: new-distiller/old-hub swallows a **405** with a WARN, and
      new-hub/old-distiller reports `null` and alerts nobody. **Distiller half DONE**
      2026-08-21 (infra took it early rather than waiting for the swap; it also
      carried `journal-day-bucketing` §11.5). Hub half: **`cchv-v0.20.0` is cut and
      its assets are verified** (2026-08-21, commit `1fb2ef30`) — only the swap on
      m4m is outstanding, and it is infra's to run.

      Everything a swap relay needs, measured rather than quoted:
      - asset `cchv-hub-0.20.0-aarch64-apple-darwin`, sha256
        `6b0cf5f6cd6960eddfc14da6a28a17b22d9312ff9da1caa0e067805915483a45`,
        57 698 048 B — agreeing across the `.sha256` sidecar and the releases API.
      - webapp entry chunk `archive-Cq5JqeN4.js` → **`archive-BLFltIAu.js`**, so the
        old chunk 404s post-swap; `cchv-v0.20.0` appears **2×** in the entry chunk
        and is the only `cchv-v[0-9]+\.[0-9]+\.[0-9]+` in it.
      - **swap-proof probe, and it is a GET**: `/v1/healthz/journal` gains the
        response fields `last_tick_at`, `ticks_last_24h`, `max_tick_age_secs`, all
        three absent from the v0.19.0 struct. Prefer this over the new
        `POST /v1/journal/ticks` route, which would prove the same thing by writing
        a row to prod — and over the `?max_tick_age_secs=` **query param**, which
        the deployed hub already answers 200 to and ignores, so sending it
        discriminates nothing (infra, 08-21). The param is honoured *and echoed
        back in the body* only post-swap; the body is the discriminator, never the
        status.
      - migration **0007** (`distiller_ticks`) is new and unapplied: `CREATE TABLE`
        + one index on an empty table, so unlike 0006's measured 6.66 s there is no
        long startup window to plan around.
      - `cchv-v0.20.0`'s Security Audit check is **red, and not a reason to hold the
        swap** — one unreachable advisory (RUSTSEC-2026-0235, `rkyv`), diagnosed in
        `AGENTS.md` § *What CI builds*. Archive/Rust/Frontend/Server-Release are all
        green on that commit.

      **The warning is a `405`, not a `404`** (infra's correction of their own probe,
      2026-08-21, and the reason it is worth the line): they read `GET
      /v1/journal/ticks` → 404 as "route absent, since a POST-only route would answer
      405" — then run 206 logged `POST /v1/journal/ticks 405`. The control settles it:
      the certainly-absent `/v1/journal/zzznope` answers GET 404 / POST 405, identical
      to `ticks`, so on this router **405 discriminates nothing** about whether a route
      exists. Verdict (old hub, route absent) unchanged; the status code in the log is
      405. Both are 4xx, so neither is retried and distillation is untouched either way.
      **Hub half DONE 2026-08-21 09:28–09:32Z** — infra swapped `cchv-v0.20.0`; migration 0007 applied 09:29:32Z (`_sqlx_migrations`). Superseded the same day by `cchv-v0.20.1` (10:10Z, migration 0008).
- [x] 8.2 Verify live: `last_tick_at` advances after a tick; `ticks_last_24h`
      sits well under 24 — infra measured **15.38 ticks/day** on this host. It is
      *not* the wake count (40–106 sleep cycles/day), but not because DarkWakes
      run no `StartInterval` agent — they do, and that claim was retracted
      2026-08-21. The cap is the interval re-arming at each run's **exit**
      (≈21.7/day awake) plus sleep coalescing missed intervals into one catch-up.
      **Verified 2026-08-21 ~13:50Z** (successor session, on m4m): `distiller_ticks` holds 15 rows since 0007, `tick_at` strictly advancing (ids 12→15: 13:11, 13:18, 13:22, 13:24Z); `/v1/healthz/journal` reports `last_tick_at` = row 15, `ticks_last_24h` = 15 — under 24, but inflated by the day's backfill runs, not an hourly-cadence reading.
- [x] 8.3 Infra's call whether the `cchv-journal` Gatus check sets
      `max_tick_age_secs`, and to what. **Answered 2026-08-21: `43200` (12 h)**,
      from a 13-day replay — median gap 68 m, p90 2 h 41 m, max 7 h 41 m (values
      corrected the same day, when the replay was re-run on tick *starts* rather
      than completions); 8 h is the tightest zero-flap value and has no headroom,
      our ~3 h 30 m would have fired 16×. Tracked `ac/infra#117`, and it goes on the URL only after 8.1
      and 8.2. Table + caveats: `docs/archive/deployment.md` §3c. Related: the
      monitoring decision on `ac/infra#94`.
