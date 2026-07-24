# distiller-self-healing — Tasks

## 1. Hub: journal staleness health endpoint

- [ ] 1.1 Add `healthz_journal` handler in `crates/hub/src/health.rs`:
      closed-day pending groups (same CTE semantics / `DAY_START_HOUR` fold as
      `journal::pending`, cross-linked by comment) joined with
      `max(messages.created_at)` per group; `grace_secs` parsed string-first
      → 400 (same pattern as `stale_after_secs`), default 7200; any stale
      group → 503 `"stale"`, else 200 `"ok"`; body lists groups
- [ ] 1.2 Register `GET /v1/healthz/journal` in `crates/hub/src/lib.rs`
      (unauthenticated, like the other healthz routes)
- [ ] 1.3 Integration tests: undrained closed day → 503; freshly dirtied
      within grace → 200; open-day-only → 200; fully drained → 200 empty;
      `grace_secs=abc|0` → 400
- [ ] 1.4 `cd src-tauri && cargo test -- --test-threads=1`, clippy
      `-D warnings`, fmt check (workspace covers `crates/hub`)

## 2. Distiller: retry + hourly ticks

- [ ] 2.1 Add `_with_retry` helper in `scripts/cchv-distill.py` (3 attempts,
      30s sleep, on `requests.RequestException` and 5xx) and wrap `pending`,
      `session_messages` page fetches, and `post_entry`; keep exit 1 on final
      failure
- [ ] 2.2 `scripts/dev.cchv.distiller.plist`: replace
      `StartCalendarInterval {05:30}` with `StartInterval 3600`; keep
      `RunAtLoad`; update the plist's explanatory comment (nightly-slot
      rationale is obsolete)
- [ ] 2.3 Verify: `--dry-run` against the live hub; simulate transient
      failure (bogus `--hub-url`) → observe 3 retries then clean non-zero
      exit, no crash-loop

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
- [ ] 4.2 Update `docs/archive/deployment.md` monitoring section with the new
      endpoint + grace semantics
