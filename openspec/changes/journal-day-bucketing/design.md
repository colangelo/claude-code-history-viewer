# Design — journal-day-bucketing

## Context

Three places compute the journal group key today, each with its own copy of the
fold expression over `sessions.first_message_time`:

| site | file | role |
|---|---|---|
| `pending` | `crates/hub/src/journal.rs:95` | the distiller's work list |
| `upsert` provenance | `crates/hub/src/journal.rs:246` | membership + coverage check on POST |
| `healthz_journal` | `crates/hub/src/health.rs:296` | staleness check |

Their comments already say they must fold identically — which is the tell that
three copies is the wrong number. This change moves all three onto one shared
fragment.

## Decision 1 — the day comes from the message, not the session

`entry_date(m) = ((m."timestamp" - day_start_hour) AT TIME ZONE 'UTC')::date`, and a
session is a member of every group for which it has at least one message. A session
therefore appears in several groups; a group's session set is the set of sessions with
a message that day.

Rejected: **`generate_series(first_day, last_day)` over the sessions table**, which
needs no message scan at all. Measured on the live archive it produces **57,661**
(session, day) pairs against 4,869 sessions — an average span of 11.8 days — because
a session resumed a week later covers every idle day in between. Those phantom groups
would each be a real pending item with an empty transcript and a wasted LLM call. The
message scan is the only exact source.

## Decision 2 — one shared SQL fragment, not three copies

`journal::SESSION_DAYS_CTE` is a `pub(crate)` `&str` holding the `msg_days` CTE, with
`$1` = `day_start_hour` and `$2` = an optional inclusive `from` date. `pending`,
`healthz_journal` and the provenance check compose it into their own statements. The
fold expression exists once.

## Decision 3 — push the date lower bound into the message scan

The `from` bound on `entry_date` is exactly equivalent to a bound on the raw timestamp:

```
entry_date >= F  ⟺  (ts - H) >= F 00:00 UTC  ⟺  ts >= F 00:00 UTC + H
```

so it can be applied to `messages."timestamp"` before any grouping. The distiller's
forward mode passes `from = today - horizon_days` (7 by default), so the hot path never
scans the archive's history.

### Measured on the live archive (pg1, 7.22 M messages / 4,882 sessions / 260 entries)

| query | shape | time |
|---|---|---|
| `pending` today | sessions-only group-by | **2.6 ms** |
| `pending` new, `from = today-7d` | message group-by, bounded | **3 250 ms** |
| `pending` new, unbounded (backfill) | message group-by | **5 430 ms** |
| `pending` new, lateral `generate_series` + `EXISTS` | narrowed to 297 candidate pairs | 3 731 ms |
| provenance check today (1 day + 1 project) | full scan | **6 587 ms** |
| provenance check new (worst group, 54 sessions) | sessions-narrowed + `EXISTS` probes | **10.8 ms** |
| single `EXISTS` probe, 67 k-message session | existing `(session_id, message_key)` index | 0.115 ms |

Two of those rows decide the design.

**The pending query gets ~1 250× slower and that is accepted.** It is an hourly
background call on the distiller's own tick; 3.25 s of pg1 CPU per hour is not a cost
worth contorting the query for. What matters is that the plan is *stable*: a parallel
seq scan bounded by the window. The lateral form narrows beautifully — 188 sessions,
297 (session, day) candidate pairs — but the planner estimates `generate_series` at
1 000 rows, mis-costs the semi-join and picks a hash join over a full `messages` scan
anyway, landing *slower* than the simple form. A plan that depends on defeating a
cardinality misestimate is a plan that regresses silently. Rejected.

(Noted, not acted on: with `enable_seqscan = off` the bounded group-by runs in
**1 076 ms** — the planner's choice is 3× worse than its own alternative at today's 29 %
window selectivity. As the archive grows the window becomes more selective and the
bitmap path wins on cost, so this self-corrects. Not worth a planner hint.)

**The provenance check gets ~600× faster and that is required.** The naive form of the
new rule — scan `messages` for one day and one project — measured 6.6 s, and the
distiller POSTs up to 50 entries per tick, which would be five minutes of pure
validation. Narrowing candidates on the sessions table first (`first_message_time <
day_end AND last_message_time >= day_start`, both already stored) leaves a handful of
sessions, each settled by one 0.115 ms `EXISTS` probe.

## Decision 4 — provenance drift is a dirty signal

A group is pending when it has no entry, **or** the entry's `session_ids` differ from
the group's computed set, **or** a session's `ingest_xid` is invisible in the entry's
snapshot (the existing rule).

This is what repairs the archive. The alternative — a one-off `--redistill` flag, or
deleting rows by hand — makes the repair a thing an operator must remember to run
correctly, and leaves no defence if provenance ever drifts again. Drift-as-dirty is
data-derived, consistent with the capability's existing "computed from data, not from a
schedule" contract, and self-healing.

For the comparison to be sound, stored provenance must be canonical. `upsert` already
sorts and dedupes into `ids` for its membership check but then binds
`payload.session_ids` — the caller's ordering — into the INSERT. All 260 stored rows
happen to be sorted (the distiller posts the pending list's order, which is sorted), so
no data repair is needed; binding `&ids` makes it guaranteed rather than lucky.

### What this costs on deploy

Over the period the distiller has actually been running forward (2026-07-04 →
2026-08-19) the correct rule yields **541** (day, project) groups. Against the 260
entries that exist:

| | groups |
|---|---|
| never had an entry at all | **287** |
| entry exists, session set wrong | **172** |
| entry exists and is already correct | 82 |

So ~85 % of the journal is currently wrong or missing, and the 287 is consequence 2 from
the proposal made concrete — whole project-days absent because no session *started*
that day.

The blast radius on deploy is bounded by the distiller's own knobs, and deliberately so:
forward mode only sees `today - 7d`, which is **59** groups, drained at 50 per hourly
tick. Repairing the rest is an explicit `--backfill --from 2026-07-04` at 20 per tick —
the operator's call, not a side effect of deploying.

## Decision 5 — the message window is a public API parameter

`GET /v1/sessions/:id/messages` gains `from` / `to`: RFC 3339, half-open `[from, to)`,
same parser and same 400 text as `/v1/search`, both optional. `X-Total-Count` counts
within the window — a total that ignored the filter would make paging wrong, which is
the bug class that header exists to prevent.

The distiller then fetches `[day_start, day_start + 1 day)` per session instead of the
whole session. For the 19/8 `infra` group that is ~7 k messages instead of 67 k, and
the 60/40 head-tail truncation finally operates on the day it claims to summarize.

## Decision 6 — migration 0006 adds `(session_id, "timestamp")`

The windowed fetch currently plans as a `BitmapAnd` of `messages_session_id_message_key_key`
and `messages_timestamp_idx` — **52.6 ms** per 500-row page, measured. The distiller
pages through every group, so this is the hot path, not the pending query. A composite
index turns it into one range scan. It also makes the provenance probes an index-only
range rather than a scan-and-filter over a session's whole key range.

Additive, `IF NOT EXISTS`, no rewrite.

## Risks

- **Empty transcript.** A group is derived from all messages, but `build_transcript`
  skips sidechain messages, so a day whose messages are all sidechain yields an empty
  transcript. The distiller posts `skip` in that case rather than calling the LLM.
  Deriving the group from non-sidechain messages instead would push a
  transcript-shaping concern into the hub's group key; keeping the hub's rule
  simple and letting the distiller judge substance matches how `skip` already works.
- **A long session is re-distilled every day it stays open,** in each of its days. That
  is correct — those days really are dirty — but a session running for a week costs a
  re-distill of each earlier day whenever the `ingest_xid` moves. The `session_ids`
  drift check does not add to this; the existing snapshot rule already does it.
- **Pending cost grows with total archive size,** not window size, while the seq-scan
  plan holds. See Decision 3; it inverts once the window is selective enough.
