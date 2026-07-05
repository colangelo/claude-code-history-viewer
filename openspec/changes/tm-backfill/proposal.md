## Why

Claude Code deletes local session history after ~30 days, but Time Machine backups
retain point-in-time copies of `~/.claude` / `~/.config/claude` going back much
further (on m4m: to 2026-01-25, i.e. sessions back to ~2025-12-26). The recovery
path was proven manually on 2026-07-05 (tifo session `7a66308e` from 2026-03-27,
recovered from the 2026-04-19 snapshot, now searchable in the hub) — but it took
an ad-hoc sequence of mounts, fake-home staging, and daemon babysitting. It must
become a documented, repeatable, per-machine operation so every machine (m4m,
ac-mbm5, and the retired ac-mbp via its attached TM disk) can backfill its own
history into the durable archive.

## What Changes

- `cchv-sync-daemon` gains a `--once` flag: run exactly one sync pass, then exit
  (exit code reflects whether the pass had errors). Removes the "run forever,
  grep the log, kill" hack from one-shot ingestion.
- `cchv-sync-daemon` gains a `CCHV_HOSTNAME` environment override for the
  identity hostname, so history restored *from another machine's* backups is
  attributed to that machine, not the machine running the ingest.
- New `scripts/tm-backfill.sh`: end-to-end CLI driver — enumerate Time Machine
  snapshots, mount each read-only, stage a fake `$HOME` (symlink into the
  snapshot, no copying), run a one-shot ingest against the live hub with correct
  machine attribution, unmount, report. Supports the local TM destination by
  default and any foreign backup store by path (e.g. ac-mbp's TM disk attached
  to another Mac).
- New runbook `docs/timemachine-backfill.md`: the operational doc future
  sessions on ANY machine follow — prerequisites, per-machine usage, foreign-
  machine recovery (ac-mbp), coverage math and known gaps, and the
  absolute-symlink trap inside backups.
- New Justfile recipe wiring the script into the repo's command surface.

## Capabilities

### New Capabilities

- `timemachine-backfill`: recovering expired agent history from Time Machine
  backups into the hub archive — snapshot enumeration/mount/unmount, isolated
  fake-home staging that never touches live config or daemon state, one-shot
  ingest with per-source-machine identity, foreign backup store support, and
  the operator runbook.

### Modified Capabilities

- `history-sync-daemon`: two new requirements — a single-pass `--once` execution
  mode with meaningful exit status, and an environment override for the
  identity hostname used in machine attribution.

## Impact

- **Code**: `crates/sync-daemon` (CLI arg parsing in `main.rs`/`lib.rs`,
  `identity.rs` hostname override); new `scripts/tm-backfill.sh`; `Justfile`;
  `docs/timemachine-backfill.md`. No hub, schema, or frontend changes.
- **Systems**: hub ingestion is reused as-is; idempotent ingest means
  re-running backfills or overlapping snapshot windows is safe. Live daemons
  and live `~/.config/claude` are never touched (throwaway state dirs under
  `~/.config/cchv/backfill/<machine>/`).
- **Constraints**: `mount_apfs`/`umount` require running unsandboxed (operator
  or agent with elevated permission); macOS-only by nature. A second-loop run
  (archive-viewer-ui) is in flight from `main` — this change deliberately stays
  out of the viewer/webui surface.
