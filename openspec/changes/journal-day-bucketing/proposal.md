# journal-day-bucketing

## Why

The 19/8 journal feed on the live archive browser contains work done on 20/8.
Reported by ac, 2026-08-20. Tracked as `ac/claude-code-history-viewer#35`.

Journal entries are grouped **per session, by the session's first message**:

```sql
-- crates/hub/src/journal.rs, `pending`
((s.first_message_time - make_interval(hours => 4)) AT TIME ZONE 'UTC')::date AS entry_date
```

So a session that starts on the 19th and keeps running through the 20th is filed
entirely under the 19th — every message it ever produces. The distiller compounds it:
`build_transcript` fetches **every** message of each session with no date filter, and
`truncate()` keeps head 60% + **tail 40%**, so the *newest* content is guaranteed to
survive into a prompt whose template says "transcripts from {entry_date}".

Measured on the live hub, 2026-08-20. The 19/8 `infra` entry's session ids include:

| session | first message (UTC) | last message (UTC) | messages |
|---|---|---|---|
| 1231594 | 2026-08-19 16:05 | **2026-08-20 22:43** | 67,088 |
| 1233018 | 2026-08-19 17:11 | **2026-08-20 20:37** | 24,029 |

That entry's `generated_at` is 2026-08-20T21:57:33Z — regenerated after a full day of
20/8 work had accumulated inside sessions labelled 19/8.

**This is the dominant case, not an edge case.** Midnight-spanning sessions are ~10% of
sessions but hold 83–92% of all messages:

| project | sessions | spanning | messages | in spanning sessions |
|---|---|---|---|---|
| infra | 200 | 20 | 842k | 775k (92%) |
| direction | 95 | 14 | 1.04M | 882k (85%) |
| cchv | 200 | 24 | 504k | 419k (83%) |

Three consequences, of which only the first was reported:

1. **Wrong day** — day N+1 work appears in day N's entry.
2. **Missing day** — day N+1's own entry omits that work entirely, because the session
   is not a member of N+1's group. Every long session hollows out the following day.
3. **Churn** — a long-running session keeps re-dirtying its start-day entry, rewriting
   it each tick with more foreign-day content and burning an LLM call each time.

The webapp is not implicated: `JournalView.tsx` groups by whatever `entry_date` the hub
returns.

## What Changes

- **The logical-day fold moves from the session to the message.** A session belongs to
  every logical day on which it has at least one message, not to the single day it
  started. `journal::pending`, the `upsert` provenance check and `health::healthz_journal`
  all derive groups from `messages."timestamp"`, through one shared SQL fragment so they
  cannot drift — the three copies of the fold expression that exist today are exactly the
  drift risk the current comments warn about.
- **`GET /v1/sessions/:id/messages` gains `from` / `to`** (RFC 3339, half-open, same
  parsing as `/v1/search`), so a caller can fetch one day's slice of a session.
  `X-Total-Count` reports the count *within the window*.
- **A stored entry whose session set no longer matches its group's computed set is
  dirty.** Provenance drift becomes a data-derived staleness signal, consistent with
  "pending is computed from data, not from a schedule". This is what re-distills the
  history already written under the old rule — once, automatically, with no
  backfill flag and no manual deletion.
- **The distiller fetches only the group's day** for each session and posts a `skip`
  row when the resulting transcript is empty, so a session with a message-less day
  (possible where a day's messages are all sidechain) cannot burn an LLM call.
- **Migration 0006 adds `messages (session_id, "timestamp")`** — the composite index the
  windowed fetch and the per-session day probes both want.

## Impact

- Specs: `journal-entries` (logical-day fold, pending list, provenance), `journal-health`
  (identical fold), `archive-search-api` (message window params).
- Code: `crates/hub/src/journal.rs`, `crates/hub/src/health.rs`,
  `crates/hub/src/browse.rs`, `migrations/0006_messages_session_timestamp.sql`,
  `scripts/cchv-distill.py`.
- **Data:** the first distiller tick after deploy re-distills every entry whose session
  set the new rule changes, and creates entries for days that previously had none. This
  is a one-time cost in LLM calls, bounded by the distiller's own `--limit` per tick.
- No frontend change. No API break: `from`/`to` are additive and optional.

## Out of scope

`DAY_START_HOUR` is applied in **UTC** while `openspec/specs/journal-entries/spec.md`
says "before `day_start_hour` **local** time". At CEST the boundary is 06:00 local, so
00:00–06:00 local work folds back a day — the same symptom from a different cause.
Fixing it re-dates the entire archive, so it is filed separately rather than smuggled
in here. This change preserves the current UTC fold exactly.
