#!/usr/bin/env bash
# Differential probe for the hourly excursion on /v1/healthz/journal.
#
# Landed in the repo after two runs on 2026-08-23 settled the cause (pg1's hourly
# WAL-checkpoint storm — docs/2026-08-23-journal-hourly-excursion.md). It is here
# because a THIRD run is already predicted: the burst phase is a fingerprint of the
# producing client and JUMPS when that client restarts (:51 -> :58 -> :04 over one
# day). When the excursion appears at a new minute, re-run this rather than
# re-deriving it, and set DENSE_FROM/DENSE_TO to bracket the new phase.
#
# THE PHASE MOVED ON 2026-08-24T11:20Z AND WILL MOVE AGAIN. A restart of the producing
# client is not the only thing that re-phases the burst: so does any change to the
# TRIGGER. max_wal_size 4 GB -> 8 GB doubled the WAL distance a forced checkpoint has to
# accumulate, so it now takes twice as long to arm -- the first post-change forced
# checkpoint ran :13:15 -> :17:04, outside the old :04-:12 window entirely (infra,
# thread 8d5eb1ba). A dense window still bracketing :04-:12 would have sampled a quiet
# stretch and read as "the storm stopped".
# So: DENSE_FROM/DENSE_TO are a CACHED DERIVATION, not a property of the system. Unless
# you have a current checkpoint-log phase to bracket, run dense across the whole hour --
#   DENSE_FROM=0 DENSE_TO=59 scripts/journal-phase-probe.sh 90
# -- and derive the phase from the run instead of assuming it.
#
# AND SINCE 2026-08-24 ~12:00Z THERE IS NO PHASE AT ALL: direction shipped its
# incremental-reconcile fix (0.99.3), pg1 measured ZERO forced checkpoints across four
# consecutive hours against a timed control still at 4/hour, and WAL generation fell
# 7-9 GB/hour -> 0.03 GB/hour (infra, thread 8d5eb1ba). Both retired windows -- :04-:12 and
# :13-:17 -- now match nothing, correctly. What this script measures today is the fold's
# own cost on a QUIET instance, which is the number any day-fold/insulation work has to be
# justified against. If an hourly excursion reappears, that is direction regressing:
# check https://<direction-prod>/api/version against 0.99.3 before re-opening anything here.
#
#   control   /v1/healthz          -> SELECT 1 on pg1 (health.rs). Same network path,
#                                     same TLS termination, same m4m process, same
#                                     connection pool, none of the day-fold.
#   treatment /v1/healthz/journal  -> the SESSION_DAYS_CTE fold over ~2.4M rows.
#
# BOTH rising together => m4m-side, or the network/connection.
# ONLY the treatment rising => pg1-side work on the fold.
# The differential is what makes this readable from a vantage point that is NOT the
# Gatus one; it cannot confirm or deny Gatus's absolute numbers and does not try to.
#
# Two design choices that are load-bearing, both from AGENTS.md:
#
#   --max-time 30 is far ABOVE any observed value (worst ever seen: 9.97 s). A ceiling
#   BELOW the query time abandons the request client-side while the server keeps
#   running it -- that is how six piled-up requests once starved each other and the
#   "reading" became a measurement of our own backlog.
#
#   The treatment cadence is 150 s, deliberately not 60 s: sampling the fold every
#   minute keeps pg1's buffer cache warm over the hot range and shrinks the very
#   excursion being measured. It is also offset from Gatus's 300 s. Expect this probe
#   to read LOWER peaks than Gatus for exactly that reason (4.15 s here vs 5.36 s
#   there, same phase, same day) -- a floor, not a discrepancy to chase.
#
#   That cuts against the full-hour default below, and knowingly. Dense-everywhere runs
#   the fold every 150 s off-phase too (it was 600 s), so it warms the cache more and
#   reads an even lower floor -- and an off-phase sample is no longer a cold-cache
#   control. Accept that while you are LOCATING an unknown phase; once you have one,
#   narrow the window back so the off-phase samples mean something again.
#
# Usage:  HUB=https://<hub-host>:8788 scripts/journal-phase-probe.sh [minutes]  (default 60)
# Env:    HUB (REQUIRED), OUT, DENSE_FROM, DENSE_TO
# Output: TSV to $OUT (default /tmp/cchv-journal-phase-probe.tsv)

set -u

# No default host: `origin` is the PUBLIC fork and internal hostnames stay out of the tree
# (the 2026-08-02 scrub). This script had re-introduced one; HEAD is clean, the history is
# not. Pass it in:  HUB=https://<hub-host>:8788 scripts/journal-phase-probe.sh 60
HUB="${HUB:?set HUB to the hub base URL, e.g. https://<hub-host>:8788}"
OUT="${OUT:-/tmp/cchv-journal-phase-probe.tsv}"
MINUTES="${1:-60}"
DENSE_FROM="${DENSE_FROM:-0}"    # minute-of-hour: dense sampling wraps from here...
DENSE_TO="${DENSE_TO:-59}"       # ...through here. DEFAULT IS THE WHOLE HOUR since
                                 # 2026-08-24: the burst phase moved off :04-:12 to
                                 # :13-:17 when max_wal_size doubled, and then the burst
                                 # stopped entirely when direction 0.99.3 shipped. A stale
                                 # window samples a quiet stretch and reads as "no storm"
                                 # whether or not there is one. Narrow it ONLY against a
                                 # phase you have just derived -- there is none today.

END=$(( $(date -u +%s) + MINUTES * 60 ))

printf 'iso\tendpoint\thttp\tsecs\n' > "$OUT"

sample() {
  local p="$1" iso code secs
  iso=$(date -u "+%Y-%m-%dT%H:%M:%SZ")
  read -r code secs < <(curl -sS -o /dev/null \
      -w '%{http_code} %{time_total}\n' --max-time 30 "$HUB$p" 2>/dev/null) \
    || { code=ERR; secs=NA; }
  printf '%s\t%s\t%s\t%s\n' "$iso" "$p" "$code" "$secs" >> "$OUT"
}

# Discard the first treatment sample when you analyse: a fresh process pays DNS + TCP
# + TLS setup that steady-state samples do not. Measured 2026-08-23 -- the first
# control of a run read 1.11 s against a 0.30-0.60 s steady state, and the first
# treatment sample of that run (5.05 s, off-phase) has never been explained.
next_treatment=0
while [ "$(date -u +%s)" -lt "$END" ]; do
  now=$(date -u +%s)
  min=$(date -u +%-M)

  sample /v1/healthz

  if [ "$now" -ge "$next_treatment" ]; then
    sample /v1/healthz/journal
    if [ "$min" -ge "$DENSE_FROM" ] || [ "$min" -le "$DENSE_TO" ]; then
      next_treatment=$(( now + 150 ))
    else
      next_treatment=$(( now + 600 ))
    fi
  fi

  sleep 60
done

printf 'DONE %s\n' "$(date -u "+%Y-%m-%dT%H:%M:%SZ")" >> "$OUT"
