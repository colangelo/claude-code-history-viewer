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
# Usage:  scripts/journal-phase-probe.sh [minutes]      (default 60)
# Env:    HUB, OUT, DENSE_FROM, DENSE_TO
# Output: TSV to $OUT (default /tmp/cchv-journal-phase-probe.tsv)

set -u

HUB="${HUB:-https://m4m.cat-bluegill.ts.net:8788}"
OUT="${OUT:-/tmp/cchv-journal-phase-probe.tsv}"
MINUTES="${1:-60}"
DENSE_FROM="${DENSE_FROM:-48}"   # minute-of-hour: dense sampling wraps from here...
DENSE_TO="${DENSE_TO:-22}"       # ...through here, to bracket the :04-:12 burst

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
