# distiller-self-healing — Tasks

## 1. Hub: journal staleness health endpoint

- [x] 1.1 Add `healthz_journal` handler in `crates/hub/src/health.rs`:
      closed-day pending groups (same CTE semantics / `DAY_START_HOUR` fold as
      `journal::pending`, cross-linked by comment) joined with
      `max(messages.created_at)` per group; bound to the forward horizon via
      `within_days` (default 7) so old un-backfilled history never pages;
      `grace_secs` (default 7200) + `within_days` parsed string-first → 400
      (same pattern as `stale_after_secs`); any in-window stale group → 503
      `"stale"`, else 200 `"ok"`; body lists in-window groups
- [x] 1.2 Register `GET /v1/healthz/journal` in `crates/hub/src/lib.rs`
      (unauthenticated, like the other healthz routes)
- [x] 1.3 Integration tests: undrained in-window closed day → 503; freshly
      dirtied within grace → 200; old day outside `within_days` → 200 (not
      listed); open-day-only → 200; fully drained → 200 empty;
      `grace_secs=abc|0`, `within_days=-1` → 400
      (+ `parse_positive` unit tests)
- [x] 1.4 `cd src-tauri && cargo test -- --test-threads=1`, clippy
      `-D warnings`, fmt check (workspace covers `crates/hub`) — 19 hub tests
      green against an ephemeral local pg16-equiv

## 2. Distiller: retry + hourly ticks

- [x] 2.1 Add `_with_retry` helper in `scripts/cchv-distill.py` (3 attempts,
      30s sleep, on connection errors/timeouts and 5xx — 4xx re-raised at once)
      and wrap `pending`, `session_messages` page fetches, and `post_entry`;
      keep exit 1 on final failure. Sleep is env-tunable
      (`CCHV_RETRY_SLEEP_SECS`) for tests/constrained windows.
- [x] 2.2 `scripts/dev.cchv.distiller.plist`: replaced
      `StartCalendarInterval {05:30}` with `StartInterval 3600`; kept
      `RunAtLoad`; rewrote the comment (DST-race + self-healing rationale).
      `plutil -lint` OK.
- [x] 2.3 Verified: transient failure (bogus hub) → 3 attempts then clean
      `FATAL`/rc≠0, no crash-loop, no traceback; live-hub happy path
      (`--horizon-days 0`) → retry-wrapped pending → "nothing pending" rc=0,
      zero LLM calls.

## 3. Release + deploy (m4m)

- [x] 3.1 Bump `package.json` to 0.13.0, `just sync-version` (+ Cargo.lock),
      quality gate (tsc, **927 vitest**, lint, i18n ✓ + Rust from §1.4),
      release commit, tag `cchv-v0.13.0`, push internal + origin — CI
      building hub asset on the fork.
- [x] 3.2 CI built+attached `cchv-hub-0.13.0-aarch64-apple-darwin` + `.sha256`
      to the `cchv-v0.13.0` release (run 30083872892, success). Relayed to
      infra (msg `694d6e40`, →home-network@m4m); **infra confirmed the swap
      2026-07-24** (reply `ea44a351`): sha256 `589ba332…` verified against
      sidecar + GitHub API digest, §2b codesign-aware sequence, hub pid 82735
      stable. `/v1/healthz` ok, `/v1/healthz/journal` → 200 `{"status":"ok"}`
      (was 404 on 0.12.0 — swap proven). Preswap backup:
      `~/.config/cchv/staging/cchv-hub-preswap-20260724-1155`.
- [x] 3.3 Installed updated distiller on m4m (`install` → `~/.local/bin`, plist
      copied, `bootout`+`bootstrap`) — `run interval = 3600s`, RunAtLoad tick
      drained pending (07-23 …, 07-22 …, 6 ok/0 failed). Hourly cadence LIVE;
      **journal feed caught up through 07-23** (the reported stall is resolved).
- [~] 3.4 DST-race fix structurally verified NOW: the reloaded hourly tick drew
      07-23 groups the old nightly run never saw, and drained them. A natural
      post-04:00-UTC tick distilling 07-24→yesterday is owed at the next
      day-close (~05:00 UTC 07-25) as belt-and-suspenders.

## 4. Monitoring (infra relay)

- [x] 4.1 Relayed to infra (same msg `694d6e40`); **infra confirmed
      2026-07-24** (reply `ea44a351`): `cchv-journal` Gatus check live on mon
      alongside cchv-hub/cchv-ingest (same IP-literal+Host-header path), URL
      `/v1/healthz/journal?within_days=7` — `within_days=7` made explicit to
      pin against server-default drift — 300s interval, ntfy alert, evaluating
      green (411ms). home-network commit `a0c6912` (gatus.yaml + mon.md 26→27
      endpoints + m4m.md hub-binary note).
- [x] 4.2 Update `docs/archive/deployment.md` monitoring section with the new
      endpoint + grace semantics (§3c hourly cadence + retry + health endpoint;
      post-swap verification checklist; reload via bootout/bootstrap)
