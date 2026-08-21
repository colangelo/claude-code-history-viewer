#!/usr/bin/env bash
# Drain the journal backfill in bounded batches, stopping on trouble rather than spinning.
#
# `cchv-distill --backfill` processes ONE page per invocation (the hub caps the pending
# list at pagination::MAX_LIMIT = 200), so repairing a real backlog means calling it
# repeatedly. This is that loop, with the stop condition the bare command has no way to
# express: two consecutive batches that distil nothing means the LLM backend is down or
# its token expired, not that the work is hard — and a naive `while` would burn the rest
# of the backlog against a dead endpoint.
#
# Usage:  just journal-backfill [FROM] [BATCH] [MAX_BATCHES]
#         scripts/journal-backfill.sh 2026-07-04 50 12
#
# Proven 2026-08-21: 401 groups, 0 failed, ~7 s/group, ~50 min wall clock.
# Expect roughly half the groups to post `skip` — most (day, project) groups are a few
# real turns surrounded by Claude Code state records (see AGENTS.md § Repo rules, #41).
set -uo pipefail

FROM="${1:-2026-07-04}"
BATCH="${2:-50}"
MAX_BATCHES="${3:-12}"
DISTILL="${CCHV_DISTILL_BIN:-$HOME/.local/bin/cchv-distill}"

[ -x "$DISTILL" ] || { echo "no distiller at $DISTILL"; exit 1; }

consecutive_idle=0
for i in $(seq 1 "$MAX_BATCHES"); do
  out=$("$DISTILL" --backfill --from "$FROM" --limit "$BATCH" 2>&1)
  printf '%s\n' "$out"

  if printf '%s' "$out" | grep -q "nothing pending"; then
    echo "== batch $i: nothing pending — backfill COMPLETE"
    exit 0
  fi

  ok=$(printf '%s' "$out" | grep -oE 'done: [0-9]+ ok' | grep -oE '[0-9]+' | tail -1)
  failed=$(printf '%s' "$out" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' | tail -1)
  ok=${ok:-0}; failed=${failed:-0}
  echo "== batch $i: ${ok} ok, ${failed} failed"

  if [ "$ok" = "0" ]; then
    consecutive_idle=$((consecutive_idle + 1))
    if [ "$consecutive_idle" -ge 2 ]; then
      echo "== STOPPING: two consecutive batches distilled nothing."
      echo "   That is a dead backend or an expired token, not slow work. Check the"
      echo "   tail above, then re-run — the hub's pending list is the state, so"
      echo "   nothing is lost by stopping."
      exit 1
    fi
  else
    consecutive_idle=0
  fi
done
echo "== batch cap reached (${MAX_BATCHES} x ${BATCH}) — re-run if groups remain"
