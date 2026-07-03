# Eval rubric — Hub ingest-freshness endpoint (`GET /v1/healthz/ingest`)

Feature: per-machine daemon-liveness endpoint so Gatus (HTTP status/body only)
can catch a dead ingest daemon even while `/v1/healthz` stays green. Judged
purely on `machines.last_seen` (a daemon heartbeat); `last_message_at` is
observability-only and never drives staleness or alerting.

All backend-observable criteria (AC1–AC5) are T2 (Rust integration tests
driving the in-process hub over HTTP). There are no frontend-observable
criteria for this feature (no UI surface), so there is no T1 file. One clause
is T3 (rubric-only, unactionable by an executable eval in the shared test db).

Eval file: `crates/loop-evals/tests/ingest-freshness_eval.rs` (5 `#[tokio::test]`s).

## Shared-test-db pollution — read this before judging AC1/AC4

The `cchv_archive_test` database is shared across every eval/gate run on this
machine and is **never truncated**: every past test suite's `machines` rows
persist forever. Confirmed at spec time: 167 of 202 rows already have
`last_seen` older than the default 7200s threshold (oldest: 9+ days). This
means the endpoint's *default-threshold, global* verdict can **never**
deterministically be "ok" — some historical machine is always stale,
regardless of what any single implementation attempt does.

- AC2/AC3 are unaffected: they either assert on a specific machine's own
  `stale` flag, or deliberately induce staleness themselves (which pre-existing
  pollution can only agree with, never contradict).
- AC1/AC4 need a global "ok" verdict to assert against. The evals get there by
  passing `stale_after_secs=999999999` (~31 years) instead of relying on the
  implicit default. This exercises the *identical* per-machine and
  aggregate-ok/stale computation the default threshold would use — just at a
  value no accumulated historical pollution can ever cross. The specific
  machine's own fields (`hostname`, `stale`, `last_seen`, `last_message_at`)
  are asserted exactly as literally specified either way.

## Criteria

### AC1 — fresh machine reports ok with full fields (T2)
One machine ingested with a fresh `last_seen` → response carries
`machine_id`, `hostname`, `last_seen` (RFC 3339), `last_message_at` (RFC
3339, since a message was ingested), `stale:false`, and (using the
pollution-safe large threshold — see above) the endpoint answers 200
`status:"ok"`.
Eval: `ac1_fresh_machine_is_ok_with_full_fields`.

### AC2 — one stale machine forces global 503 (T2)
Two machines ingested; one backdated 3h via the permitted raw-SQL exception.
At the default threshold this alone guarantees global staleness regardless of
other db contents. Endpoint returns 503 `status:"stale"`; the backdated
machine's entry is `stale:true`; the untouched machine's entry is
`stale:false`.
Eval: `ac2_one_stale_machine_triggers_503_others_stay_accurate`.

### AC3 — `stale_after_secs` is honored and validated (T2)
A machine backdated 3h is `stale:true` at the default (implicit) 7200s
threshold, and flips to `stale:false` for that same machine when queried with
`?stale_after_secs=14400` (the eval intentionally does not assert the
*overall* status at 14400s, since that's subject to the same shared-db
pollution as AC1/AC4 and isn't what this criterion is about). Non-numeric
(`abc`) and non-positive (`0`, `-100`) values all return 400.
Eval: `ac3_stale_after_secs_threshold_is_honored_and_validated`.

### AC4 — staleness ignores message recency (T2)
A machine with a fresh `last_seen` but zero ingested messages reports
`last_message_at:null`, `stale:false`, and (via the same pollution-safe large
threshold) the endpoint answers 200.
Eval: `ac4_zero_messages_reports_null_last_message_at_and_not_stale`.

### AC5 — unauthenticated, matching `/v1/healthz` policy (T2)
A GET with no `Authorization` header never gets 401/403 — it gets a real
answer (200 or 503, whichever the current global state warrants). The eval
asserts `status ∈ {200, 503}`, which both encodes "not 401/403" and forces a
genuine pre-implementation failure (unmodified app 404s on the missing
route, which satisfies neither).
Eval: `ac5_no_auth_header_required`.

### T3 — empty archive → 200 "ok" (rubric only, no eval)
Spec requirement: an archive with **no machines at all** must return 200
`status:"ok"` with an empty `machines` array (bootstrap must not alarm).
**Not testable in this environment**: the shared integration-test database
accumulates machines from every suite/run and is never empty (202 rows and
counting as of writing) — there is no way for a T2 eval to observe the
`machines` table in a truly empty state without truncating shared data other
suites depend on, which the RUNBOOK's raw-SQL exception explicitly does not
permit (only backdating `last_seen` is allowed).
**Verify by code review**: confirm the query path returns `200 ok` with
`machines: []` when the `machines` table LEFT JOIN yields zero rows (i.e. the
"any machine stale → 503" logic is only reached when the machines list is
non-empty; an empty list must short-circuit to `ok`), rather than e.g.
defaulting to 503 on an empty aggregate or dividing by a zero count anywhere.
