#!/usr/bin/env bash
# Differential probe for the hourly :05/:10 excursion on /v1/healthz/journal.
#
# Control  : /v1/healthz          -> SELECT 1 on pg1. Same network path, same pool,
#                                    none of the day-fold. Cheap enough to sample often.
# Treatment: /v1/healthz/journal  -> the SESSION_DAYS_CTE fold over ~2.4M rows.
#
# If BOTH rise at :05, the contention is m4m-side or on the network/connection.
# If ONLY the journal endpoint rises, it is pg1-side work on the fold.
#
# --max-time is 30s: far above any observed value (max seen 9.97s), so it never
# fires in practice. A ceiling BELOW the query time would abandon the request
# client-side while the server kept running it -- self-inflicted backlog, the
# exact mistake recorded in AGENTS.md.
#
# Journal cadence is 150s, deliberately NOT 60s: sampling the fold every minute
# would keep pg1's buffer cache warm over the hot range and could mask the very
# excursion being measured. 150s is offset from Gatus's 300s.

set -u
H="https://m4m.cat-bluegill.ts.net:8788"
OUT=/tmp/cchv-journal-phase-probe.tsv
END=$(( $(date -u +%s) + 3600 ))   # ~17:22Z

printf 'iso\tendpoint\thttp\tsecs\n' > "$OUT"

sample() {
  local path="$1" iso code secs
  iso=$(date -u "+%Y-%m-%dT%H:%M:%SZ")
  read -r code secs < <(curl -sS -o /dev/null \
      -w '%{http_code} %{time_total}\n' --max-time 30 "$H$path" 2>/dev/null) \
    || { code=ERR; secs=NA; }
  printf '%s\t%s\t%s\t%s\n' "$iso" "$path" "$code" "$secs" >> "$OUT"
}

next_journal=0
while [ "$(date -u +%s)" -lt "$END" ]; do
  now=$(date -u +%s)
  min=$(date -u +%-M)

  sample /v1/healthz

  # Dense over the excursion window (:48 -> :22 of the next hour), sparse before.
  if [ "$now" -ge "$next_journal" ]; then
    sample /v1/healthz/journal
    if [ "$min" -ge 48 ] || [ "$min" -le 22 ]; then
      next_journal=$(( now + 150 ))
    else
      next_journal=$(( now + 600 ))
    fi
  fi

  sleep 60
done

printf 'DONE %s\n' "$(date -u "+%Y-%m-%dT%H:%M:%SZ")" >> "$OUT"
