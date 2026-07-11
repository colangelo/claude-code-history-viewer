# Acceptance evals — Hub journal-entries surface (issue #12)

Scope: **hub surface only** (`crates/hub` + `migrations/`). Distiller script,
launchd plist, webapp UI, and any frontend/i18n work are OUT of scope.

Every acceptance criterion is **backend/HTTP-observable** → all **T2** (Rust
integration tests). No T1 (no frontend code in scope) and no T3 (every
criterion is executable). All 11 live in the single file
`crates/loop-evals/tests/journal-entries_eval.rs`, one `#[tokio::test]` per
criterion, driven over HTTP against an in-process hub (`hub::MIGRATOR` +
`hub::router` on `127.0.0.1:0`), seeded only through `POST /v1/ingest`.

## Tier summary

| AC | Tier | Test fn |
|----|------|---------|
| AC1 | T2 | `ac1_pending_lists_closed_groups_ordered_and_limited` |
| AC2 | T2 | `ac2_open_day_excluded_closed_day_listed` |
| AC3 | T2 | `ac3_logical_day_fold_across_the_4utc_boundary` |
| AC4 | T2 | `ac4_post_stores_entry_browse_returns_it_pending_clears` |
| AC5 | T2 | `ac5_repost_replaces_and_keeps_single_row` |
| AC6 | T2 | `ac6_late_session_dirties_entry_then_repost_clears` |
| AC7 | T2 | `ac7_invalid_payloads_rejected_and_store_nothing` |
| AC8 | T2 | `ac8_skip_row_hidden_from_browse_and_search` |
| AC9 | T2 | `ac9_default_scope_returns_messages_and_journal` |
| AC10 | T2 | `ac10_scope_messages_and_journal` |
| AC11 | T2 | `ac11_auth_is_enforced` |

## JSON contract the evals assert (implementer must conform)

The openspec capability specs fix behavior but not wire field names; the evals
pin these concrete names. Implement to these:

- **`GET /v1/journal/pending?from=YYYY-MM-DD&limit=N`** (read-auth). Top-level
  JSON **array**, newest-first by `entry_date`. Each group:
  `{ "entry_date": "YYYY-MM-DD", "project_path": <str>, "session_ids": [<surrogate int>] }`.
  `from` = date lower bound; `limit` caps the page. Groups whose logical date
  is the current open day are excluded.
- **`POST /v1/journal/entries`** (machine-token, same auth as `/v1/ingest`).
  Body: `{ entry_date, project_path, status: "entry"|"skip", headline, summary,
  topics: [str], open_questions: [str], session_ids: [int], model }`. Success
  → **2xx**. Validation failure → **400/422** (a client error that is *not*
  404/405), with **no partial write**. `skip` bodies omit content fields.
- **`GET /v1/journal/entries?project=<project_path>&limit=N`** (read-auth).
  Top-level JSON **array** of `entry`-status rows, newest-first, each with
  `entry_date, project_path, headline, summary, topics, open_questions,
  session_ids, generated_at, model`. `project` filter matches `project_path`.
  Skip rows never appear.
- **`GET /v1/search?q=&scope=all|messages|journal`**. `scope` defaults to
  `all`. `all`/`journal` add a top-level **`journal`** array (separate from the
  unchanged message `results` array); each journal hit carries at least
  `{ entry_date, project_path, session_ids, headline }` plus ranking fields.
  `scope=messages` returns the pre-change shape with **no `journal` key**.

## Isolation model (shared, never-truncated test DB)

Each test uses a **fresh random `project_path` + hostname**; all presence/
absence assertions filter the global response to this test's `project_path`
(pollution can never create that path). Groups use **recent, closed** logical
dates (a few days before now) and pending queries pass `from = <earliest date>`
to exclude the fixed-2026 data other suites ingest. Search tests seed a
per-test random FTS lexeme (`zqterm…`). No raw SQL; seeding is via
`POST /v1/ingest`. Dirty-detection needs no backdating — see AC6.

## Criteria

### AC1 — pending lists closed groups, ordered, limited (T2)
Ingest two sessions for one project on two closed past days (D_old < D_new).
`GET /v1/journal/pending` (read-auth) lists **both** `(entry_date,
project_path)` groups filtered to this project; D_new precedes D_old
(newest-first); each group carries its own surrogate session id. `?limit=1`
caps the returned array to length 1.

### AC2 — open logical day excluded (T2)
A session first-messaged **now** falls on the current open logical day and MUST
NOT appear in pending, while a closed-day group from the same project does. The
expected open date is computed in-test as `(now − 4h).date` (default
`day_start_hour = 4`, UTC).

### AC3 — logical-day fold across the 04:00 UTC boundary (T2)
With default `day_start_hour = 4 UTC`: a session first-messaged `02:30Z` on
D+1 folds back to D, and one first-messaged `23:00Z` on D stays on D. They form
**a single** pending group dated D carrying **both** session ids.

### AC4 — write + browse + pending clears (T2)
`POST /v1/journal/entries` (machine token) stores an `entry`; `GET
/v1/journal/entries` returns it with headline, summary, 3 topics, non-empty
open_questions, and the session id; the group then disappears from pending.

### AC5 — upsert replaces, single row (T2)
Re-posting the same `(entry_date, project_path)` with changed
headline/summary/topics replaces the entry — browse shows the new content and
**exactly one** row for the group.

### AC6 — late session dirties, re-post clears (T2)
After an entry exists (clean), ingesting an **additional** session for the same
group bumps `sessions.updated_at` past the entry's `generated_at`, so the group
becomes pending again. Re-posting clears it. No backdating: real wall-clock
ordering (POST then ingest) suffices.

### AC7 — invalid payloads rejected atomically (T2)
Three invalid `POST`s each return a **client validation error (400/422, not
404/405)**: (a) `entry` with 2 topics, (b) `entry` referencing a nonexistent
session id, (c) an unknown `status` value. Afterward browse for the project is
empty — nothing was stored.

### AC8 — skip row hidden everywhere (T2)
A `skip`-status POST (2xx) removes the group from pending and the skip row
appears in **neither** `GET /v1/journal/entries` **nor** any `/v1/search`
response (no journal hit references the project).

### AC9 — default-scope search returns both blocks (T2)
For a term seeded into both an archived message and a stored entry, default
`/v1/search` returns the existing message `results` (message hit for the
session present) **and** an additive `journal` array whose hit carries
`headline`, `entry_date`, `project_path`, and the session id.

### AC10 — scope=messages / scope=journal (T2)
Same query: `scope=messages` returns a response with **no `journal` key** while
still returning the message hit; `scope=journal` returns the entry hit and
reports **no message hits** (`results` absent or empty).

### AC11 — auth enforced (T2)
`GET /v1/journal/pending` and `GET /v1/journal/entries` without an
`Authorization` bearer are **401**; `POST /v1/journal/entries` is **401** both
without a token and with an invalid token.

## Red-state verification (pre-implementation)

Against the unmodified crate all 11 tests FAIL at runtime for the correct
reason — the `/v1/journal/*` routes and the `scope` journal block do not exist,
so requests return 404 (route missing) instead of the asserted
200/2xx/401/400. Confirmed:

```
SQLX_OFFLINE=true TEST_DATABASE_URL=postgres://ac@localhost/cchv_archive_test \
  cargo nextest run -p loop-evals --test journal-entries_eval --profile loop --no-fail-fast
# → 11 tests run: 0 passed, 11 failed
```

Compiles against the unmodified crate (no references to yet-to-exist symbols;
all new behavior driven over HTTP) and is clippy-pedantic/`-D warnings` clean.
