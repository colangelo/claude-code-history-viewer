## Context

Claude Code prunes local history after ~30 days. Time Machine keeps APFS
snapshots of the whole disk, each holding a rolling ~30-day window of session
files at that point in time. The union of snapshots is gap-free wherever
consecutive backups are less than 30 days apart.

The manual recovery flow was proven end-to-end on 2026-07-05 on m4m:

1. `tmutil listbackups` / `tmutil destinationinfo` to enumerate.
2. `mount_apfs -o ro -s com.apple.TimeMachine.<STAMP>.backup <device> <mnt>`.
3. Session data at `<mnt>/<STAMP>.backup/<Volume> - Data/Users/<user>/.config/claude/projects/`.
   **Trap:** `.claude` inside the backup is an *absolute* symlink to
   `/Users/ac/.config/claude` — following it reads the LIVE system.
4. Stage a fake `$HOME` whose `.claude` points at the restored data; run
   `cchv-sync-daemon` with `HOME=<fake>` and a throwaway `DAEMON_CONFIG`
   (live hub URL/token, isolated `state_dir` seeded with the real
   `machine_id`); wait for `sync pass complete`; kill.
5. Hub dedups (idempotent ingest), so overlapping snapshot windows and re-runs
   are free.

Result: 4 sessions (main + 3 subagents), 178 messages, 0 errors; searchable
with correct 2026-03-27 timestamps and machine attribution.

Constraints:
- Live daemon state (`~/.claude-history-sync/`) and live `~/.config/claude`
  must never be touched.
- `mount_apfs`/`umount` cannot run inside the seatbelt sandbox.
- A second-loop run (archive-viewer-ui) is in flight from `main`; this change
  must stay surgical: `crates/sync-daemon`, `scripts/`, `docs/`, `Justfile`.
- ac-mbp (retired MacBook, used until ~2026-04/05) has no cchv deployment; its
  history must be recoverable from its TM disk attached to any Mac that has
  the daemon binary, with attribution to ac-mbp rather than the host machine.

## Goals / Non-Goals

**Goals:**
- One documented command per machine that backfills any/all TM snapshots into
  the hub, runnable unattended and safely re-runnable.
- Correct machine provenance for foreign-machine restores (ac-mbp case).
- Zero interference with live Claude Code, live daemon, or its checkpoints.
- Runbook good enough that a future session on any machine can execute the
  recovery without re-deriving the mount/staging mechanics.

**Non-Goals:**
- Non-macOS backup sources (rsync archives, Backblaze, etc.) — the fake-home
  staging pattern generalizes, but only TM is scripted here.
- Recovering non-Claude providers from backups (Codex/OpenCode/… don't
  auto-delete; nothing is lost by scoping staging to `.claude`). The staging
  helper is written so other provider roots could be added later.
- HFS+ `Backups.backupdb`-era stores (pre-APFS Time Machine). Out of scope
  until a real disk of that era shows up.
- Automating discovery of *where* ac-mbp's TM disk lives.

## Decisions

**D1 — daemon `--once` flag instead of run-and-kill.**
`run()` already performs an immediate pass before entering its loop; `--once`
runs that single pass and exits, with exit code 0 only if `stats.errors == 0`.
Alternative considered: keep the "grep the log and kill" wrapper — rejected as
fragile (log format coupling, orphaned processes on script death) and useless
for exit-status-based scripting. `--once` also skips watcher setup entirely
(no point watching a read-only snapshot).

**D2 — `CCHV_HOSTNAME` env override for identity hostname.**
`Identity::load_or_create` takes the hostname from `gethostname()`; for
foreign restores the records must carry the source machine's hostname (e.g.
`ac-mbp`). An env var matches the existing `CCHV_INGEST_*` override pattern
and avoids touching the config schema. The `machine_id` side needs no code
change: it already comes from `<state_dir>/machine_id`, so per-source state
dirs give stable per-machine ids.

**D3 — stage via symlink into the read-only mount, not copy.**
`$FAKE/home/.claude` → `<snapshot>/…/Users/<user>/.config/claude` (or
`…/.claude` for stock layouts — the script resolves which one is a real
directory, never following the in-backup `.claude` symlink). The daemon only
reads, so ro is fine; original mtimes are preserved; no disk cost. Alternative
(cp -Rp, as in the manual proof) rejected: slow and pointless at
whole-projects scale.

**D4 — per-source-machine state dirs under `~/.config/cchv/backfill/<machine>/`.**
Each source machine label gets its own `machine_id` + checkpoint, stable
across runs and snapshots. For the local machine the `machine_id` is seeded
by *copying* `~/.claude-history-sync/machine_id` (same identity as the live
daemon → history merges under the existing machine row). For a foreign label
the daemon generates one on first run and it persists. The checkpoint in that
state dir is per-snapshot-useless (every snapshot has different paths) but
harmless — and within one multi-snapshot run it dedups identical file
size+mtime combos cheaply. Alternative (always-fresh temp state dir) rejected:
would generate a new machine UUID per run for foreign machines, fragmenting
provenance.

**D5 — one script, subcommand-ish flags, config derived from the live daemon.toml.**
`tm-backfill.sh` reads `hub_url`/`hub_token` from `~/.config/cchv/daemon.toml`
(the skill-documented source of truth; survives the pg1 URL migration) and
writes a temp `daemon.toml` per run. Flags: `--list`, `--snapshot <stamp>`,
`--all`, `--store <path>` (foreign TM store root; default = `tmutil
destinationinfo` mount point), `--machine <label>` (default: local hostname
behavior, no `CCHV_HOSTNAME` set), `--user <name>` (home dir name inside the
backup, default `ac`), `--dry-run` (mount + report what would be ingested,
no hub writes). Snapshot mounts go under the user's cache dir, unmounted in a
trap on exit.

**D6 — mount by store, not by `tmutil listbackups`.**
`tmutil listbackups` only covers the *active* destination; a foreign disk
attached for ac-mbp recovery isn't the active destination. The script instead
finds the backup store's APFS device (`diskutil info -plist <store-path>`)
and enumerates `diskutil apfs listSnapshots` filtered to
`com.apple.TimeMachine.*`, which works identically for local and foreign
stores. `--list` prints stamps + which ones hold a claude dir.

**D7 — runbook lives in `docs/timemachine-backfill.md` (repo), not a skill.**
The repo is git+Syncthing-synced to every machine, so the doc travels with the
script it documents; a machine-local skill would not. The `cchv-find` skill
gets a one-line pointer (separate, outside this change's repo surface).

## Risks / Trade-offs

- [`mount_apfs` needs unsandboxed execution; agents may hit permission walls]
  → script fails loudly with the exact command to run manually; runbook says
  to expect the prompt.
- [Foreign-machine daemon binary availability (ac-mbp recovery host may lack
  `cchv-sync-daemon`)] → runbook documents: run from any machine that has the
  binary with the foreign disk attached; binary also buildable via
  `cargo build --release -p sync-daemon`.
- [Snapshot layout drift across macOS eras (`<Volume> - Data` naming, home at
  `Macintosh HD - Data/Users/<user>`)] → script globs for `*- Data/Users/<user>`
  and falls back to `Users/<user>` (older layouts); `--list` surfaces
  what it found per snapshot, so drift is visible before ingest.
- [In-backup `.claude` absolute symlink silently escapes to the live disk]
  → script uses `.config/claude` when it is a real directory, else `.claude`
  only if it is a real directory; symlinks are never followed for root
  selection (checked with `-d` + not `-L`).
- [Coverage gaps are invisible: consecutive backups >30 days apart lose the
  head of the window (m4m: sessions last touched 2026-03-17..19 are gone)]
  → `--list` prints inter-snapshot gaps >30d as warnings; runbook documents
  the math and m4m's known hole.
- [Hub reachability from the recovery host (tailnet-only)] → script curls
  `/v1/healthz` before mounting anything and aborts early with a clear error.
- [Ingesting observer/noise projects inflates the archive] → accepted: parity
  with the live daemon (which ingests them too); hub dedups re-runs. Filtering
  is a hub-side concern, not a backfill-side one.

## Migration Plan

Pure addition — no schema, hub, or config migrations. Rollback = delete the
script/doc/flag; ingested history stays (the archive is cumulative by design).
Deploy of the new daemon binary to other machines follows the existing scp +
`codesign -f -s -` + `launchctl kickstart -k` procedure, but is only needed
where backfill will run (the flag is unused by the launchd daemon).

## Open Questions

- Where is ac-mbp's TM disk today, and does it hold snapshots through
  2026-04/05? (Discovery is an operator step in the runbook; the tooling is
  ready either way.)
- Are there pre-2026 TM disks (for the June-2025 target) and are they APFS-era?
  HFS+ stores would need a `Backups.backupdb` staging variant (explicitly out
  of scope until one exists).
