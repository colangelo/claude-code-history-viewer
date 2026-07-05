# Time Machine history backfill

Recover Claude Code sessions that were deleted by its ~30-day local retention,
from Time Machine backups, into the hub archive. Proven end-to-end on
2026-07-05 (tifo session `7a66308e` from 2026-03-27, recovered from m4m's
2026-04-19 snapshot, searchable in the hub with correct timestamps and
attribution).

**Run this on each machine, against that machine's own backups.** The hub
dedups everything (idempotent ingest), so overlapping snapshot windows,
re-runs, and interrupted runs are all safe. For a machine that no longer runs
(e.g. ac-mbp), attach its Time Machine disk to any Mac with the daemon binary
and use `--store` + `--machine` (see below).

## TL;DR

```bash
# What's recoverable? (mounts each snapshot briefly; warns on >30d gaps)
just tm-backfill --list

# Ingest one snapshot
just tm-backfill --snapshot 2026-04-19-192710

# Ingest everything on this machine's TM destination (oldest first)
just tm-backfill --all

# Retired machine's disk attached to this Mac
just tm-backfill --list --store /Volumes/backup-ACMBP
just tm-backfill --all  --store /Volumes/backup-ACMBP --machine ac-mbp
```

`just tm-backfill …` = `scripts/tm-backfill.sh …`. The commands that mount
snapshots (`--list`, `--snapshot`, `--all`) must run **unsandboxed** —
`mount_apfs` fails inside an agent sandbox; grant the permission or run the
printed mount command manually.

## Prerequisites

- macOS with the backup disk mounted (`tmutil destinationinfo` shows the
  active destination; a foreign disk just needs to appear under `/Volumes`).
- `~/.config/cchv/daemon.toml` with `hub_url` + `hub_token` (the script reads
  these; never hardcodes — the hub URL will change when it moves off m4m).
- Hub reachable (script preflights `GET /v1/healthz` before mounting anything).
- `cchv-sync-daemon` ≥ the version with `--once` (2026-07-05). Default lookup:
  `~/.local/bin/cchv-sync-daemon`, then `target/release/cchv-sync-daemon`;
  build with `cargo build --release -p sync-daemon`, or pass `--daemon-bin`.
- `jq` not required; `diskutil`/`plutil`/`curl` are stock macOS.

## How it works

Each Time Machine snapshot is an APFS snapshot of the whole disk, holding
whatever `~/.config/claude/projects/` contained at backup time — i.e. a
rolling ~30-day window of sessions. Per snapshot the script:

1. Mounts it read-only: `mount_apfs -o ro -s com.apple.TimeMachine.<STAMP>.backup <device> <mnt>`.
2. Locates the backed-up home (`*/ - Data/Users/<user>`, falling back to
   `*/Users/<user>` for older layouts) and picks the Claude root: a **real**
   `.config/claude` directory, else a **real** `.claude` directory.
3. Stages an isolated fake `$HOME` whose `.claude` symlinks into the snapshot
   (no copying), with a temp `daemon.toml` pointing at the live hub but at a
   **dedicated state dir** — the live daemon and live `~/.config/claude` are
   never touched.
4. Runs `cchv-sync-daemon --once` (single pass, exit code reflects errors).
5. Unmounts. Subagent transcripts (`<session>/subagents/*.jsonl`) are picked
   up by the parser along with the main sessions.

### ⚠ The symlink trap

On our machines `.claude` inside a backup is an **absolute symlink to
`/Users/ac/.config/claude`** — reading through it inside a mounted snapshot
lands on the **live** filesystem, not the backup. The script only accepts
real directories (never symlinks) when picking the Claude root. If you ever
poke around a snapshot by hand, use the `.config/claude` path explicitly.

## Machine attribution

| Case | Invocation | Identity used |
|------|-----------|---------------|
| This machine's own backups | (default) | Live daemon's `machine_id` (seeded into `~/.config/cchv/backfill/local/`) + real hostname → history merges under this machine's existing archive row |
| Another machine's disk | `--machine <label>` | Persistent per-label `machine_id` under `~/.config/cchv/backfill/<label>/` (generated on first run, stable ever after) + `CCHV_HOSTNAME=<label>` |

Pick one label per source machine and stick with it (`ac-mbp`, not sometimes
`ac-mbp-2019`) — the label's state dir IS the identity. If the source machine
still boots, you can instead run the whole backfill *on it* with no
`--machine` flag; it only needs the binary, the daemon.toml, and its TM disk.

## Coverage math (what's recoverable)

A snapshot taken on day D covers sessions last touched in `[D-30, D]`
(Claude Code prunes by file mtime). The union of snapshots is gap-free iff
consecutive snapshots are **< 30 days apart**; a gap of G > 30 days loses
sessions last touched in the first `G - 30` days of the gap. `--list` warns
about such gaps.

Known state of the fleet (2026-07-05):

- **m4m**: snapshots from 2026-01-25 → recoverable back to ~2025-12-26.
  One hole: 2026-03-16 → 2026-04-19 is a 34-day gap, so sessions last touched
  **2026-03-17..19 are unrecoverable** on this disk.
- **ac-mbm5**: run `--list` there.
- **ac-mbp** (retired ~2026-04/05): its TM disk must be located and attached
  somewhere; then `--store <path> --machine ac-mbp --user <homedir-name>`.
  History earlier than any disk's oldest snapshot (e.g. the June-2025 target)
  needs older rotated TM disks, if they exist.

## Verifying a recovery

```bash
HUB=$(grep hub_url  ~/.config/cchv/daemon.toml | sed 's/.*= *"\(.*\)"/\1/')
TOK=$(grep hub_token ~/.config/cchv/daemon.toml | sed 's/.*= *"\(.*\)"/\1/')
# sessions for a recovered project (name or path):
curl -s -H "Authorization: Bearer $TOK" "$HUB/v1/sessions?project=<name>" | jq .
# or full-text search for a phrase you remember:
curl -s -H "Authorization: Bearer $TOK" "$HUB/v1/search?q=<words>&limit=5" | jq .
```

Check `message_count > 0` and that `first/last_message_time` match the era
you restored. Hub `message_count` below the local JSONL line count is normal
(non-message record types aren't counted).

## Troubleshooting

| Symptom | Cause / fix |
|---------|-------------|
| `mount_apfs failed … Resource busy` | Snapshot already mounted (earlier run died before cleanup). `mount \| grep TimeMachine`, unmount the stray mountpoint, re-run. |
| `mount_apfs` fails immediately under an agent | Sandbox. Run the script unsandboxed or execute the printed mount command manually. |
| `hub not reachable` | Tailnet down, or hub URL moved — re-check `~/.config/cchv/daemon.toml`. |
| `no home for user <u>` on every snapshot | Wrong `--user` (home dir name inside the backup) or non-APFS-era store. HFS+ `Backups.backupdb` stores are not supported by this script. |
| Ingest exits non-zero | Some file failed to parse or a batch failed; checkpoint didn't advance for the failed file — fix/ignore and re-run, dedup makes it free. Parse failures worth keeping should become Gitea issues on the repo. |
| Same session ingested from two snapshots | Expected and fine: identical messages dedup; a session that *grew* between snapshots gets its later messages added by the later snapshot. Order doesn't matter. |

## Housekeeping

- Backfill state lives in `~/.config/cchv/backfill/<machine>/` (machine_id +
  checkpoint). Deleting a *foreign* label's dir loses that identity —
  subsequent runs would create a new machine row in the archive. The `local`
  dir is safely regenerable (re-seeded from the live daemon).
- The archive is cumulative: nothing you do here can delete archived data,
  and unmounting/removing backups after ingestion is safe.
