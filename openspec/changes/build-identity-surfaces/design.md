## Context

See `proposal.md` — *Why*. Current state that shapes the approach:

- `crates/hub/src/health.rs::healthz` returns a literal `json!({"status":"ok","db":"up"})`;
  `healthz_journal` fills `JournalHealthResponse` from a `TickSummary` row read by a
  LATERAL query over `distiller_ticks` (newest row + 24 h count, one round trip).
- `crates/hub/src/journal.rs::record_tick` takes `TickPayload { mode, groups_pending }`,
  validates both, inserts, prunes inline. Machine-token auth.
- `migrations/0007_distiller_ticks.sql` — `distiller_ticks(id, tick_at, mode,
  groups_pending)`; `0008` is the latest migration.
- `scripts/cchv-distill.py` is a PEP 723 `uv run --script`; `Hub.record_tick` posts the
  tick with a WARN-and-continue failure policy; `main()` logs `backend=… model=…` as its
  first line, *after* secrets resolution. `scripts/test_cchv_distill.py` has stub-hub
  tests for the tick record already (`test_tick_record_states_its_mode_and_work_list_size`,
  `test_dry_run_records_no_tick`).
- `scripts/sync-version.cjs` rewrites two files from `package.json` with regex/JSON;
  `AGENTS.md` § Release Process Phase 3 stages four paths explicitly and its Guard 1
  (`git diff --quiet`) fails closed if a fifth target is changed but unstaged.
- The distiller is deployed as a **copy** (`install` from `git cat-file`, by infra); the
  hub as a release asset swapped by infra (`cchv-deploy` skill, `deployment.md` §2b).
- Workspace crates inherit `version.workspace = true`, so `env!("CARGO_PKG_VERSION")`
  in the hub crate is the `package.json` version after `just sync-version`.

## Goals / Non-Goals

**Goals:**
- One unauthenticated GET proves a hub swap for every release, route or no route.
- One GET says whether *both* halves of a release (hub + distiller) are live.
- The distiller's log names the exact file that ran, comparably to `git` without ssh.
- Nothing a current monitor asserts changes shape; a pre-change distiller keeps working.

**Non-Goals:**
- Alerting on version skew (Gatus pins are infra's call and must move with the swap).
- Embedding a git commit in the hub binary (a build-time `vergen`-style dependency for
  a value the release version already pins; the tag → sha map is in git).
- Changing how the distiller is installed. Symlink is rejected (below); a versioned
  install path is a later, separate decision (#40's second option).
- A webapp version-chip ↔ API cross-check (useful, not this change).

## Decisions

1. **Hub version from `CARGO_PKG_VERSION`, not a new source of truth.** It is already
   `package.json`'s value via `sync-version`; a release that forgets `sync-version`
   ships a wrong version *everywhere*, which Guard 2 (the `Cargo.lock` census) already
   catches. *Alternative:* read `package.json` at runtime — the hub has no such file on
   the host.

2. **Distiller release version is a `sync-version` target, not a hand-bumped constant.**
   A constant bumped "when the script changes" is exactly the forgotten step this change
   exists to expose; the cost of the sync approach is that **every release dirties the
   installed copy by one line**, i.e. a false "stale" that costs infra a one-minute
   reinstall. That is the fail-closed direction, and the blob id makes the false
   positive cheap to dismiss (`git diff <old-tag> <new-tag> -- scripts/cchv-distill.py`
   shows one line). Implementation: the script carries a marker line
   `DISTILL_VERSION = "0.20.1"  # sync-version` and `sync-version.cjs` rewrites it with
   a regex anchored on the marker, **exiting 1 if the marker is missing** (a silent
   no-op would reintroduce the stale-copy class at the tooling layer).

3. **Exact identity is the script's own git blob id, computed at start.**
   `hashlib.sha1(b"blob %d\0" % len(data) + data)` over `Path(__file__).read_bytes()` is
   by definition `git hash-object scripts/cchv-distill.py`, so the check a reader runs
   is `git rev-parse <rev>:scripts/cchv-distill.py` — the same comparison the
   *installed-script-is-a-copy* rule already prescribes, made readable from the log and
   the API. Zero maintenance, survives `install` (bytes are preserved), and it is the
   value that discriminates at commit granularity where the version discriminates at
   release granularity. *Alternative:* sha256 of the file — equally exact but not
   comparable to anything git prints without a second command.

4. **Identity is logged before any network call, in every mode.** The first log line
   becomes `cchv-distill <version> blob=<12 hex> mode=<forward|backfill|dry-run>`, emitted
   before secrets resolution — so a run that dies on a bao/op failure still says who it
   was. The existing `backend=… model=…` line stays second. Both values ride the tick
   payload; `--dry-run` still posts nothing (unchanged contract).

5. **Tick columns are nullable; the hub validates shape, not provenance.**
   `0009_distiller_ticks_identity.sql`: `ALTER TABLE distiller_ticks ADD COLUMN
   distiller_version text, ADD COLUMN distiller_blob text` (no defaults, no backfill —
   historical rows legitimately have no identity). `TickPayload` gains
   `Option<String>` for both with `#[serde(default)]`; `distiller_blob`, when present,
   must match `^[0-9a-f]{40}$` or the record is a 400 naming the field. The version is
   not validated against the hub's own (skew is a *reading*, not an error — the whole
   point is to see it). *Alternative:* one `distiller_identity` JSONB column — opaque to
   the LATERAL query and to anyone reading the table by hand.

6. **`healthz_journal` reports identity alongside liveness and never alerts on it.**
   `TickSummary` and the LATERAL query gain the two columns; `JournalHealthResponse`
   gains `hub_version: &'static str`, `last_tick_distiller_version: Option<String>`,
   `last_tick_distiller_blob: Option<String>`. The verdict logic is untouched. A
   `hub_version` ahead of `last_tick_distiller_version` *after a tick* is the reading
   "the distiller half is still the old copy"; before a tick it says nothing, which the
   deploy doc must state (the first tick after a swap is what makes the read valid).

7. **Symlink install stays rejected.** The worktree is Syncthing-shared; a symlink
   makes a peer's uncommitted edit production on the next tick, with no deploy boundary
   and no trace. The copy is correct; it only needed to say what it is.

## Risks / Trade-offs

- [Every release now shows a distiller "version behind" until infra reinstalls] →
  the relay for any release lists the reinstall as part of the deploy (it already does
  when the script changed); the doc names the one-line diff as the way to tell a
  version-only skew from a real one; the blob is the tie-breaker.
- [`sync-version.cjs` regex misses a reformatted marker] → it exits non-zero on a
  missing marker, and Guard 1 in the release recipe catches a changed-but-unstaged
  script; a unit check in `scripts/test_cchv_distill.py` asserts `DISTILL_VERSION`
  equals `package.json`'s version, so `just test-run`'s Python half fails before the
  release does.
- [`read_bytes()` of `__file__` differs from the installed bytes — e.g. a future
  wrapper or bytecode cache] → `uv run --script` executes the file itself; the test
  compares the computed blob against `git hash-object` on the same path, and a wrapper
  would fail it.
- [A monitor pins `version` and fires on the next release] → non-goal here, but the
  spec text and the deploy doc carry infra's widen-before/narrow-after rule and the
  exact-semver rule; the relay for this release says so explicitly.
- [Migration on a live hub] → `ALTER TABLE … ADD COLUMN` without default on a
  hundreds-of-rows table is instantaneous; 0007/0008 already proved the startup
  migration path. Rollback: the old binary ignores the columns; the new distiller's
  extra payload fields hit an old hub as unknown JSON keys — serde ignores them by
  default on `TickPayload`, so either order of the two halves is safe (the same
  property 0007 was designed for).

## Migration Plan

1. Land code + migration + tests + `sync-version` + `AGENTS.md` edits in one commit
   series on `main`; archive-tests, rust-tests and the Python tests green.
2. Release `cchv-v0.21.0` per `AGENTS.md` § Release Process — the explicit `git add`
   list now includes `scripts/cchv-distill.py`, and Guard 1/2 run unchanged.
3. Relay to infra (`cchv-deploy`): hub swap from the release asset **and** distiller
   reinstall from `git cat-file cchv-v0.21.0:scripts/cchv-distill.py`. Swap proof:
   `GET /v1/healthz` → `"version":"0.21.0"`. Both-halves proof: after the first tick,
   `GET /v1/healthz/journal` → `hub_version == last_tick_distiller_version == "0.21.0"`
   and `last_tick_distiller_blob == $(git rev-parse cchv-v0.21.0:scripts/cchv-distill.py)`.
   Expected non-200s: none — additive fields only.
4. Rollback: swap the previous asset back; the columns stay, harmlessly null-filled
   from then on.

## Open Questions

- Whether `hub_version` should also appear on `/v1/healthz/stats` and `/v1/healthz/ingest`
  for symmetry. Cheap, but nobody asked; can be added without touching the specs above
  if a consumer wants it.
