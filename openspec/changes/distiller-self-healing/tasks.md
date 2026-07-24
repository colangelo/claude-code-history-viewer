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

- [ ] 3.1 Bump `package.json` to 0.13.0, `just sync-version`, quality gate
      (Phase 1 of Release Process), commit, tag `cchv-v0.13.0`, push
      internal + origin
- [ ] 3.2 Build hub, stage in `~/.config/cchv/staging/`, relay binary swap to
      infra per `docs/archive/deployment.md` §2b; verify
      `/v1/healthz/journal` live (200/503 as appropriate) after swap
- [ ] 3.3 Install updated distiller: copy `scripts/cchv-distill.py` →
      `~/.local/bin/cchv-distill`, install plist, `launchctl bootout` +
      `bootstrap`; verify next tick logs (idle → "nothing pending", or
      drains)
- [ ] 3.4 Observe one natural post-04:00-UTC tick distilling yesterday
      (closes the DST-race verification)

## 4. Monitoring (infra relay)

- [ ] 4.1 Relay to infra (home-network): add Gatus check for
      `GET /v1/healthz/journal` (Host-header pattern like cchv-hub /
      cchv-ingest checks); confirm green once deployed
- [x] 4.2 Update `docs/archive/deployment.md` monitoring section with the new
      endpoint + grace semantics (§3c hourly cadence + retry + health endpoint;
      post-swap verification checklist; reload via bootout/bootstrap)
