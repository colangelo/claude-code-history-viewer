# Run report: journal-entries

**Repo:** /Users/ac/_sync/dev/claude-code-history-viewer
**Spec:** specs/journal-entries.md
**Status:** needs-human
**Started:** 2026-07-11T11:39:06.762Z  **Finished:** 2026-07-11T12:32:09.655Z

**Claude cost (counterfactual API value, billed to subscription):** $17.2249

**Error:** Review rounds exhausted without approval.

## Eval plan

| Criterion | Tier | Text |
|---|---|---|
| AC1 | T2 | (T2) After ingesting sessions for a closed past day, `GET /v1/journal/pending` (read-auth) lists that (entry_date, project_path) group carrying its session ids, newest-first across groups, and honors `?limit=`. |
| AC2 | T2 | (T2) A session whose first message is stamped now (current open logical day) does not appear in pending, while the closed-day group from the same project does. |
| AC3 | T2 | (T2) With default day_start_hour 4 UTC, a session first-messaged 02:30 UTC on day D+1 and one first-messaged 23:00 UTC on day D (same project) appear as a single pending group dated D. |
| AC4 | T2 | (T2) `POST /v1/journal/entries` (machine token) stores an entry; `GET /v1/journal/entries` returns it with headline, summary, topics, open_questions, and session ids; the group disappears from pending. |
| AC5 | T2 | (T2) Re-POSTing the same (entry_date, project_path) with changed content replaces the entry — browse shows the new content and exactly one row for that group. |
| AC6 | T2 | (T2) After an entry exists, ingesting an additional session for the same group makes the group pending again; re-POSTing clears it again. |
| AC7 | T2 | (T2) Invalid POSTs return 4xx and store nothing: entry status with 2 topics, entry referencing a nonexistent session id, and an unknown status value. |
| AC8 | T2 | (T2) A skip-status POST removes the group from pending, and the skip row appears in neither `GET /v1/journal/entries` nor any `/v1/search` response. |
| AC9 | T2 | (T2) `/v1/search` with default scope, for a term seeded into both an archived message and a stored entry, returns the existing message `results` AND a `journal` array whose hit carries headline, entry_date, project, and session ids. |
| AC10 | T2 | (T2) For that same query, `scope=messages` returns a response with no `journal` key, and `scope=journal` returns the entry hit while reporting no message hits. |
| AC11 | T2 | (T2) Without an Authorization bearer, `GET /v1/journal/pending` and `GET /v1/journal/entries` are rejected as unauthorized, and `POST /v1/journal/entries` is rejected without a valid machine token. |

## Review rounds

### Round 1 — changes requested

- **blocker** `crates/hub/src/journal.rs`: `entry_search_text` indexes only headline, summary, topics, and open_questions. The spec requires journal FTS to cover entry_date, project, and session ids as well, so `/v1/search` cannot find journal rows by those required fields. Include the date, project_path, and session_ids in `search_text` for entry rows.
- **blocker** `crates/hub/src/journal.rs`: `POST /v1/journal/entries` accepts empty `session_ids` because existence validation is skipped for an empty array. The journal contract requires session provenance and at least one archived session; an empty-id entry or skip can clear pending without any drill-down provenance. Reject empty `session_ids` for both `entry` and `skip` payloads.
- **major** `crates/hub/src/ingest.rs`: Dirty detection depends on `sessions.updated_at`, but message-only ingests for an existing session do not bump it: the touched-session aggregate update refreshes counts and first/last timestamps only. A late message batch after a journal entry can therefore fail to make the group pending again. Update `sessions.updated_at` when touched sessions gain messages, or base pending dirty detection on inserted message timestamps.
- **major** `migrations/0002_journal_entries.sql`: The schema does not enforce `status IN ('entry','skip')`. The migration contract calls for that status domain, and an invalid direct row would still suppress pending because the pending join treats any journal row as a watermark. Add a CHECK constraint for the allowed statuses.
### Round 2 — changes requested

- **blocker** `crates/hub/src/journal.rs`: `POST /v1/journal/entries` accepts an omitted or null `model` for `status: "entry"` and stores it as NULL. The spec says each entry MUST record the model that generated it. Add non-empty model validation for entry rows, and consider a conditional DB check in `migrations/0002_journal_entries.sql` so invalid rows cannot bypass the API.
- **major** `crates/hub/src/journal.rs`: Dirty detection can miss a concurrent late ingest. Pending uses `sessions.updated_at > journal_entries.generated_at`, but both timestamps are written with PostgreSQL `now()` transaction-start time. If an ingest transaction starts before an entry POST and commits after it, the newly ingested session can have `updated_at < generated_at`, so the group is treated as clean even though data arrived after the entry. Use a commit/statement-order-safe watermark such as `clock_timestamp()` at the session update/upsert and journal upsert points, or a monotonic ingest version, then compare that watermark.
### Round 3 — changes requested

- **blocker** `crates/hub/src/journal.rs`: `POST /v1/journal/entries` only verifies that `session_ids` exist, not that they belong to the posted `(entry_date, project_path)` group or cover the group being watermarked. A caller can post an entry/skip for project A/date D with an existing session id from project B, and pending will clear project A/date D because the row key matches. Validate joined `sessions`/`projects` logical date and project_path for every id, and reject mismatched or incomplete provenance.
- **major** `crates/hub/src/ingest.rs`: Dirty detection still treats no-op duplicate session ingest as new journal data: the existing session upsert updates `sessions.updated_at = now()` even when all messages conflict and `touched_sessions` is empty. After an entry or skip row exists, replaying the same archived batch can make the group pending again despite no new session data. Use a separate journal dirty watermark, or only advance the dirty watermark when messages/session content materially change.
- **major** `crates/hub/src/ingest.rs`: The `clock_timestamp()` watermark is stamped before the ingest transaction commits, so it does not actually encode commit order. If an ingest updates a session timestamp, then a journal POST writes a later `generated_at`, then the ingest commits, pending compares `latest_ingest < generated_at` and misses data that became visible after the entry. Serialize journal writes against in-flight ingests for the group or use a commit-order/locked watermark that cannot be older than the commit that publishes the new data.

## Deterministic gate


## Browser verification


## Commits

- 67f4f29 frozen evals
- 17548ce implement
- 57ef850 fix round 1
- f48d449 fix round 2
