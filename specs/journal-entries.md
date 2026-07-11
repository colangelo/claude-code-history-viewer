# Hub journal-entries: schema, pending/write/browse API, search integration

## Description

Add the hub side of the journal-entries distillation stage (issue #12). Full
contract: `openspec/changes/journal-entries/` (proposal, capability specs,
design) — committed on main and available in this worktree; this spec scopes
the loop run to the **hub surface only**. The distiller script, launchd plist,
webapp UI, and any frontend/i18n work are explicitly OUT of scope for this run.

What to build (in `crates/hub` + `migrations/`):

- **Migration `migrations/0002_journal_entries.sql`** (additive only — no
  existing table touched): `journal_entries` keyed unique
  `(entry_date, project_path)`, columns per the openspec capability spec —
  `status` (`entry`|`skip`), headline, summary, `topics TEXT[]`,
  `open_questions TEXT[]`, session provenance (hub surrogate session ids),
  `model`, `generated_at`, `search_text` + generated tsvector + GIN index.
  Entries fold **across machines** by `project_path`.
- **`GET /v1/journal/pending`** (read-auth): data-derived work list of
  (entry_date, project_path) groups needing distillation — groups with
  archived sessions but no journal row, or with session data ingested after
  the row's `generated_at` (compare against `sessions.updated_at`). Each group
  carries its `entry_date`, `project_path`, and the hub surrogate session ids
  (the distiller needs them; no second lookup). Newest-first; `limit` and
  `from` (date lower bound) params. Groups whose logical date equals the
  current open day are excluded. Logical date of a session = date of
  `first_message_time` shifted by `day_start_hour` (default **4**, applied in
  **UTC** by default; both may be config knobs but MUST default as stated and
  MUST NOT require config changes to existing constructions).
- **`POST /v1/journal/entries`** (machine-token auth, same model as
  `/v1/ingest`): validated upsert by `(entry_date, project_path)`. Reject with
  4xx and no partial write: `entry` status with topics outside 3–8 or missing
  headline/summary, session ids that don't exist, unknown status. `skip`
  status stores a watermark row (group key + session ids + `generated_at`,
  no content).
- **`GET /v1/journal/entries`** (read-auth): browse `entry`-status rows,
  filterable by project and date range, newest-first, paginated, full content
  including session ids. Skip rows never surface here.
- **`/v1/search`**: add `scope` param (`all` default | `messages` |
  `journal`). Scope `all`/`journal` adds a **separate additive `journal`
  array** (ranked FTS hits over entry text: headline, summary, topics,
  open_questions, entry_date, project, session ids) next to the existing
  `results`; `scope=messages` returns exactly the pre-change shape (no
  `journal` key). Skip rows never match. The existing message-hit shape and
  ordering MUST NOT change at any scope.

Implementation constraints:

- **Use runtime-checked sqlx** (`sqlx::query` / `sqlx::query_as` with manual
  mapping) for all new queries — the gate builds with `SQLX_OFFLINE=true` and
  sqlx-cli is NOT installed, so new `sqlx::query!` macro calls cannot get
  `.sqlx` metadata regenerated and will fail the offline build.
- Keep every existing public construction compiling unchanged: hub tests and
  frozen evals build `hub::AppState` / config and call `hub::router` — new
  knobs must be optional with defaults, additive fields must not break struct
  construction patterns in `crates/hub/tests/` and `crates/loop-evals/tests/`.
- Follow existing hub module conventions: new `crates/hub/src/journal.rs`
  sibling to `search.rs`/`ingest.rs`, routes in `lib.rs::router`, errors via
  `HubError`. Clippy pedantic `-D warnings` + rustfmt clean.

Eval mechanics (T2, `loop-evals`): spawn the in-process hub per the RUNBOOK
(`hub::MIGRATOR` + `hub::router` on `127.0.0.1:0`, fresh random machine
ids/tokens), seed sessions/messages via `POST /v1/ingest` with chosen message
timestamps (past dates → closed days; a first message stamped "now" → open
day). Requests to the new `/v1/journal/*` routes against the unmodified crate
fail 404/405 at runtime — the correct RED. Dirty-detection needs no
backdating: POST the entry (its `generated_at` is now), then ingest more data
for the group (bumps `sessions.updated_at` past it).

## Acceptance Criteria

- (T2) After ingesting sessions for a closed past day, `GET /v1/journal/pending` (read-auth) lists that (entry_date, project_path) group carrying its session ids, newest-first across groups, and honors `?limit=`.
- (T2) A session whose first message is stamped now (current open logical day) does not appear in pending, while the closed-day group from the same project does.
- (T2) With default day_start_hour 4 UTC, a session first-messaged 02:30 UTC on day D+1 and one first-messaged 23:00 UTC on day D (same project) appear as a single pending group dated D.
- (T2) `POST /v1/journal/entries` (machine token) stores an entry; `GET /v1/journal/entries` returns it with headline, summary, topics, open_questions, and session ids; the group disappears from pending.
- (T2) Re-POSTing the same (entry_date, project_path) with changed content replaces the entry — browse shows the new content and exactly one row for that group.
- (T2) After an entry exists, ingesting an additional session for the same group makes the group pending again; re-POSTing clears it again.
- (T2) Invalid POSTs return 4xx and store nothing: entry status with 2 topics, entry referencing a nonexistent session id, and an unknown status value.
- (T2) A skip-status POST removes the group from pending, and the skip row appears in neither `GET /v1/journal/entries` nor any `/v1/search` response.
- (T2) `/v1/search` with default scope, for a term seeded into both an archived message and a stored entry, returns the existing message `results` AND a `journal` array whose hit carries headline, entry_date, project, and session ids.
- (T2) For that same query, `scope=messages` returns a response with no `journal` key, and `scope=journal` returns the entry hit while reporting no message hits.
- (T2) Without an Authorization bearer, `GET /v1/journal/pending` and `GET /v1/journal/entries` are rejected as unauthorized, and `POST /v1/journal/entries` is rejected without a valid machine token.
