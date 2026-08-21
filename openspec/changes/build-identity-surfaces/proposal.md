## Why

Nothing in the archive stack announces which build it is. `GET /v1/healthz` returns
`{"db":"up","status":"ok"}` with no version and the hub binary embeds no `cchv-v*`
string, so a hub swap can only be proven by *route archaeology* — a probe whose form
took two corrections on the 2026-08-21 `cchv-v0.20.0` swap and was vacuous for
`cchv-v0.20.1` (no new route; the migration had to be the proof). The distiller is
worse: `~/.local/bin/cchv-distill` is an installed **copy** of
`scripts/cchv-distill.py` that logs nothing about itself, so on the same day a Jul-24
copy ran for hours against an Aug-21 `main` while every checkbox and CI badge said
the fix was live (Gitea #39, #40; `docs/2026-08-21-journal-day-bucketing.md` rules
2–3; `AGENTS.md` § Repo rules). Both halves of a release deploy through different
hands (infra swaps the hub, reinstalls the distiller), and today there is no single
reading that says whether both landed.

## What Changes

- `GET /v1/healthz` gains `"version"` — the hub's `CARGO_PKG_VERSION`, which
  `just sync-version` already derives from `package.json`. A swap is then proven by
  one unauthenticated GET, for every release, including ones that add no route.
- `scripts/cchv-distill.py` announces its identity at the top of every tick:
  - a `DISTILL_VERSION` constant that `scripts/sync-version.cjs` maintains as its
    **fifth target** (alongside `Cargo.toml`, `tauri.conf.json`, and the lockfile the
    recipe refreshes separately) — so the release ceremony's explicit-staging list
    and its Guard 1 grow by one path, in this change;
  - its own **git blob id**, computed at start from the bytes of the running file
    (`sha1("blob <len>\0" + bytes)`), so `git rev-parse HEAD:scripts/cchv-distill.py`
    against the logged value *is* the `cmp`-against-HEAD check, readable from the log
    or the API without touching the host.
- `POST /v1/journal/ticks` accepts two **optional** fields, `distiller_version` and
  `distiller_blob` (40 lowercase hex). Migration `0009` adds both as nullable columns on
  `distiller_ticks`; a pre-change distiller posts neither and reads back `null`, which
  is itself the reading "an old distiller is ticking".
- `GET /v1/healthz/journal` reports `hub_version`, `last_tick_distiller_version` and
  `last_tick_distiller_blob` next to `last_tick_at`, so "did both halves of the release
  land" becomes one GET instead of a hub probe plus an `ssh` and a `cmp`.
- Not a breaking change: every new field is additive; old clients ignore them and old
  distillers keep working.

Rejected, with the reason recorded in `design.md`: installing the distiller by symlink
into the repo (the worktree is Syncthing-shared, so a peer's in-flight edit would go
live on the next tick with no review step); inferring identity from
`journal_entries.generated_at` (moves only when a tick had work *and* succeeded, the
same ambiguity the tick record already closed); a hand-bumped script version (the
forgotten bump is the exact failure this change exists to make visible).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `archive-search-api`: the health endpoint requirement — `GET /v1/healthz` SHALL
  report the hub's version.
- `journal-health`: the journal staleness endpoint SHALL report the hub version and
  the last tick's distiller identity alongside tick liveness.
- `journal-entries`: the Distiller job SHALL announce its version and blob id at tick
  start and carry both on the tick record; the Distiller tick record endpoint SHALL
  accept and store them, validating the blob's shape.

## Impact

- **Hub** (`crates/hub`): `health.rs` (`healthz`, `healthz_journal` + its tick query
  and response struct), `journal.rs` (`TickRecord`, `record_tick`), new
  `migrations/0009_distiller_ticks_identity.sql`. Tests in `crates/hub/tests/read_test.rs`
  and the journal/tick tests.
- **Distiller** (`scripts/cchv-distill.py`, `scripts/test_cchv_distill.py`): version
  constant, blob computation, first log line, tick payload.
- **Release tooling**: `scripts/sync-version.cjs` (fifth target, fails loudly if the
  marker is missing), `AGENTS.md` § Release Process Phase 3 (explicit `git add` list
  + Guard 1 prose), `AGENTS.md` § Version Management diagram.
- **Deploy**: a hub swap **and** a distiller reinstall, relayed to infra per the
  `cchv-deploy` skill — and the swap proof for this release is
  `GET /v1/healthz` → `version == <released>`, the first release that needs no route
  probe. The skill's swap-proof section and `docs/archive/deployment.md` §2b/§3c get
  the new proof; Gatus may *report* the version but MUST NOT pin it (infra's
  widen-before/narrow-after rule), and any comparison is exact semver, never a
  substring (`0.18.1` is a prefix of `0.18.10`).
- **Monitoring consumers** (infra, `cchv-journal` Gatus check, `ac/infra#117`): new
  fields only; nothing they assert today changes shape.
