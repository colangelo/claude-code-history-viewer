#!/usr/bin/env bash
# tm-backfill.sh — recover expired agent history from Time Machine backups
# into the cchv hub archive.
#
# Claude Code deletes local history after ~30 days; each TM snapshot holds a
# rolling ~30-day window of it. This script mounts snapshots read-only, stages
# an isolated fake $HOME pointing into the snapshot, and runs
# `cchv-sync-daemon --once` against the live hub. The hub's idempotent ingest
# makes re-runs and overlapping snapshot windows free.
#
# Runbook: docs/timemachine-backfill.md
#
# Usage:
#   tm-backfill.sh --list [--store <path>] [--user <name>]
#   tm-backfill.sh --snapshot <stamp> [--store <path>] [--machine <label>] [--user <name>] [--dry-run]
#   tm-backfill.sh --all [--store <path>] [--machine <label>] [--user <name>] [--dry-run]
#
# Options:
#   --list             List snapshots: date, claude-dir presence, coverage gaps.
#   --snapshot STAMP   Ingest one snapshot (e.g. 2026-04-19-192710).
#   --all              Ingest every snapshot in the store (oldest first).
#   --store PATH       Backup store root (a mounted TM volume, e.g.
#                      /Volumes/backup-M4M or a foreign machine's TM disk).
#                      Default: the active TM destination (tmutil destinationinfo).
#   --machine LABEL    Source-machine label for attribution. Default: local
#                      (reuses the live daemon's machine id + hostname).
#                      A foreign label gets its own persistent machine id under
#                      ~/.config/cchv/backfill/<LABEL>/ and CCHV_HOSTNAME=<LABEL>.
#   --user NAME        Home directory name inside the backup (default: ac).
#   --dry-run          Mount + report what would be ingested; no hub writes.
#   --daemon-bin PATH  cchv-sync-daemon binary (default: ~/.local/bin/cchv-sync-daemon,
#                      falling back to target/release/cchv-sync-daemon).
#
# Requirements: macOS, mount_apfs (must run unsandboxed), diskutil, jq,
# a reachable hub configured in ~/.config/cchv/daemon.toml.
set -euo pipefail

CONFIG="${CCHV_DAEMON_TOML:-$HOME/.config/cchv/daemon.toml}"
BACKFILL_ROOT="$HOME/.config/cchv/backfill"
LIVE_STATE_DIR="$HOME/.claude-history-sync"
RETENTION_DAYS=30

MODE="" STORE="" STAMP="" MACHINE="" BK_USER="ac" DRY_RUN=0 DAEMON_BIN=""

die() { echo "ERROR: $*" >&2; exit 1; }
log() { echo "[tm-backfill] $*" >&2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --list) MODE=list ;;
    --all) MODE=all ;;
    --snapshot) MODE=one; STAMP="${2:?--snapshot needs a stamp}"; shift ;;
    --store) STORE="${2:?--store needs a path}"; shift ;;
    --machine) MACHINE="${2:?--machine needs a label}"; shift ;;
    --user) BK_USER="${2:?--user needs a name}"; shift ;;
    --dry-run) DRY_RUN=1 ;;
    --daemon-bin) DAEMON_BIN="${2:?--daemon-bin needs a path}"; shift ;;
    -h|--help) sed -n '2,36p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
  shift
done
[ -n "$MODE" ] || die "one of --list / --snapshot <stamp> / --all is required (see --help)"

# ---------- store + device resolution ----------

resolve_store() {
  if [ -n "$STORE" ]; then
    [ -d "$STORE" ] || die "store path not found: $STORE"
    return
  fi
  STORE=$(tmutil destinationinfo 2>/dev/null | awk -F' : ' '/^Mount Point/{print $2; exit}')
  [ -n "$STORE" ] && [ -d "$STORE" ] \
    || die "no mounted TM destination found; pass --store <path> (is the backup disk attached?)"
}

resolve_device() {
  DEVICE=$(diskutil info -plist "$STORE" 2>/dev/null \
    | plutil -extract DeviceIdentifier raw -o - - 2>/dev/null) \
    || die "cannot resolve APFS device for $STORE"
  DEVICE="/dev/$DEVICE"
}

# Chronologically sorted TM snapshot stamps (e.g. 2026-04-19-192710).
list_stamps() {
  diskutil apfs listSnapshots "$DEVICE" 2>/dev/null \
    | awk '/Name: +com\.apple\.TimeMachine\./{print $NF}' \
    | sed -E 's/^com\.apple\.TimeMachine\.([0-9-]+)\.backup$/\1/' \
    | sort
}

# ---------- snapshot mounting ----------

MNT=""
# NOTE: don't gate the unmount on grepping the mount table — mktemp returns
# /var/folders/… while mount(8) lists /private/var/folders/…, so a path match
# silently skips the unmount and the next mount of the same snapshot fails
# with "Resource busy". Just try; suppressed failures are harmless.
cleanup() {
  if [ -n "$MNT" ]; then
    umount "$MNT" 2>/dev/null || diskutil unmount force "$MNT" >/dev/null 2>&1 || true
    rmdir "$MNT" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

mount_snapshot() { # $1 = stamp; sets MNT
  local stamp="$1"
  MNT=$(mktemp -d "${TMPDIR:-/tmp}/tm-backfill-mnt.XXXXXX")
  if ! mount_apfs -o ro -s "com.apple.TimeMachine.${stamp}.backup" "$DEVICE" "$MNT" >/dev/null 2>&1; then
    rmdir "$MNT"; MNT=""
    die "mount_apfs failed for $stamp — run unsandboxed, or manually:
  mount_apfs -o ro -s com.apple.TimeMachine.${stamp}.backup $DEVICE <mountpoint>"
  fi
}

unmount_snapshot() {
  cleanup
  MNT=""
  trap cleanup EXIT INT TERM
}

# Locate the backed-up home dir inside a mounted snapshot; empty if absent.
find_backup_home() {
  local d
  for d in "$MNT"/*.backup/*" - Data/Users/$BK_USER" "$MNT"/*.backup/*/Users/"$BK_USER"; do
    [ -d "$d" ] && { echo "$d"; return; }
  done
}

# Claude history root inside a backed-up home. NEVER follows symlinks:
# in this fleet's backups, `.claude` is an ABSOLUTE symlink into the LIVE
# filesystem of the restore host — following it would ingest live data as
# backup data. Only real directories qualify.
find_claude_root() { # $1 = backup home
  local d
  for d in "$1/.config/claude" "$1/.claude"; do
    if [ -d "$d" ] && [ ! -L "$d" ] && [ -d "$d/projects" ]; then
      echo "$d"; return
    fi
  done
}

# ---------- listing ----------

do_list() {
  log "store: $STORE ($DEVICE), backup user: $BK_USER"
  local prev_epoch=0 stamp date_part epoch gap_days home root nproj
  local stamps; stamps=$(list_stamps)
  [ -n "$stamps" ] || die "no Time Machine snapshots found on $DEVICE"
  while IFS= read -r stamp; do
    date_part=${stamp%-*}
    epoch=$(date -j -f "%Y-%m-%d" "$date_part" +%s 2>/dev/null || echo 0)
    if [ "$prev_epoch" -gt 0 ] && [ "$epoch" -gt 0 ]; then
      gap_days=$(( (epoch - prev_epoch) / 86400 ))
      if [ "$gap_days" -gt "$RETENTION_DAYS" ]; then
        echo "  !! gap of ${gap_days}d — history last touched in the first $((gap_days - RETENTION_DAYS))d of it is unrecoverable"
      fi
    fi
    prev_epoch=$epoch
    mount_snapshot "$stamp"
    home=$(find_backup_home)
    root=""; nproj="-"
    if [ -n "$home" ]; then
      root=$(find_claude_root "$home")
      [ -n "$root" ] && nproj=$(find "$root/projects" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
    fi
    if [ -n "$root" ]; then
      echo "$stamp  claude: yes (${nproj} project dirs, ${root#"$MNT"/})"
    elif [ -n "$home" ]; then
      echo "$stamp  claude: NO (home found, no claude dir)"
    else
      echo "$stamp  claude: NO (no home for user $BK_USER)"
    fi
    unmount_snapshot
  done <<< "$stamps"
}

# ---------- ingesting ----------

read_toml_value() { # $1 = key
  grep -E "^$1[[:space:]]*=" "$CONFIG" | head -1 | sed 's/.*= *"\(.*\)"/\1/'
}

prepare_identity() { # sets STATE_DIR and HOSTNAME_OVERRIDE
  HOSTNAME_OVERRIDE=""
  if [ -z "$MACHINE" ]; then
    STATE_DIR="$BACKFILL_ROOT/local"
    mkdir -p "$STATE_DIR"
    if [ ! -f "$STATE_DIR/machine_id" ]; then
      [ -f "$LIVE_STATE_DIR/machine_id" ] \
        || die "no $LIVE_STATE_DIR/machine_id — is the live daemon set up? (or pass --machine <label>)"
      cp -p "$LIVE_STATE_DIR/machine_id" "$STATE_DIR/machine_id"
      log "seeded local backfill identity from the live daemon's machine_id"
    fi
  else
    STATE_DIR="$BACKFILL_ROOT/$MACHINE"
    mkdir -p "$STATE_DIR"   # daemon generates + persists a machine_id on first run
    HOSTNAME_OVERRIDE="$MACHINE"
  fi
}

check_hub() {
  HUB_URL=$(read_toml_value hub_url); HUB_TOKEN=$(read_toml_value hub_token)
  [ -n "$HUB_URL" ] && [ -n "$HUB_TOKEN" ] || die "hub_url/hub_token not found in $CONFIG"
  curl -sf --max-time 10 "$HUB_URL/v1/healthz" >/dev/null \
    || die "hub not reachable at $HUB_URL (tailnet up?)"
}

resolve_daemon_bin() {
  if [ -z "$DAEMON_BIN" ]; then
    for c in "$HOME/.local/bin/cchv-sync-daemon" \
             "$(cd "$(dirname "$0")/.." && pwd)/target/release/cchv-sync-daemon"; do
      [ -x "$c" ] && { DAEMON_BIN="$c"; break; }
    done
  fi
  [ -n "$DAEMON_BIN" ] && [ -x "$DAEMON_BIN" ] \
    || die "cchv-sync-daemon binary not found; build with 'cargo build --release -p sync-daemon' or pass --daemon-bin"
}

ingest_snapshot() { # $1 = stamp
  local stamp="$1"
  mount_snapshot "$stamp"
  local home; home=$(find_backup_home)
  if [ -z "$home" ]; then
    log "$stamp: no home for user $BK_USER — skipping"
    unmount_snapshot; return
  fi
  local root; root=$(find_claude_root "$home")
  if [ -z "$root" ]; then
    log "$stamp: no claude history — skipping"
    unmount_snapshot; return
  fi

  local nproj
  nproj=$(find "$root/projects" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
  if [ "$DRY_RUN" = 1 ]; then
    log "$stamp: DRY RUN — would ingest ${nproj} project dirs from ${root#"$MNT"/}"
    unmount_snapshot; return
  fi

  # Isolated fake home: .claude is a symlink into the read-only snapshot.
  local fake; fake=$(mktemp -d "${TMPDIR:-/tmp}/tm-backfill-home.XXXXXX")
  ln -s "$root" "$fake/.claude"
  cat > "$fake/daemon.toml" <<EOF
hub_url = "$HUB_URL"
hub_token = "$HUB_TOKEN"
scan_interval_secs = 3600
state_dir = "$STATE_DIR"
providers_exclude = ["crush", "aider"]
EOF

  log "$stamp: ingesting ${nproj} project dirs (machine: ${MACHINE:-local})"
  local rc=0
  HOME="$fake" DAEMON_CONFIG="$fake/daemon.toml" CCHV_HOSTNAME="$HOSTNAME_OVERRIDE" \
    "$DAEMON_BIN" --once || rc=$?
  rm -rf "$fake"
  unmount_snapshot
  if [ "$rc" -ne 0 ]; then
    # Per-session failures (parse errors, rejected batches) are logged by the
    # daemon above; the rest of the snapshot ingested fine and the checkpoint
    # didn't advance for the failed files, so a re-run is safe and cheap.
    # In --all mode keep sweeping — one bad session must not strand the
    # remaining snapshots.
    FAILED_STAMPS+=("$stamp")
    log "$stamp: ingest pass reported errors (exit $rc) — continuing; see daemon WARN lines above"
    return
  fi
  log "$stamp: done"
}

# ---------- main ----------

resolve_store
resolve_device

case "$MODE" in
  list)
    do_list
    ;;
  one|all)
    [ "$DRY_RUN" = 1 ] || check_hub
    [ "$DRY_RUN" = 1 ] || resolve_daemon_bin
    prepare_identity
    FAILED_STAMPS=()
    if [ "$MODE" = one ]; then
      list_stamps | grep -qx "$STAMP" || die "snapshot $STAMP not found on $DEVICE (see --list)"
      ingest_snapshot "$STAMP"
    else
      local_stamps=$(list_stamps)
      [ -n "$local_stamps" ] || die "no Time Machine snapshots found on $DEVICE"
      while IFS= read -r s; do ingest_snapshot "$s"; done <<< "$local_stamps"
    fi
    if [ "${#FAILED_STAMPS[@]}" -gt 0 ]; then
      log "backfill finished WITH ERRORS in: ${FAILED_STAMPS[*]} (machine: ${MACHINE:-local}, store: $STORE) — re-run those stamps after triage; dedup makes it cheap"
      exit 1
    fi
    log "backfill complete (machine: ${MACHINE:-local}, store: $STORE)"
    ;;
esac
