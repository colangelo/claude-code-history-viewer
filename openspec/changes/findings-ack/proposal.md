## Why

`GET /v1/healthz/findings` counts every finding in its window, and nothing can mark one
as dealt with (Gitea #76). A finding that has been triaged keeps Gatus `cchv-findings`
red for the whole window, and while it is red a new leak cannot page. The two ways out
both cost something: rewriting rows falsifies the record, and shortening the Gatus
window hides real findings with the triaged one. This happened on 2026-10-21 (false
positives) and again on 2026-10-24 (one real match, redacted at ingest, triaged by the
client-data owner).

The daily step-change alert (ac/security#14) also needs per-day counts to compare a day
against the 7-day median, and the 2026-10-22 archive sweep wrote 1,142 findings in one
20-second window that such a median must not read as new leaks.

## What Changes

- New table `finding_acks` (migration 0014): one row per acknowledged finding, with its
  classification (`false_positive` or `handled`), who acknowledged it, a note and a time.
  `credential_findings` itself is never rewritten.
- New CLI `hub findings ack`, run on the hub host like `findings redact`. It selects
  findings by message (row id or `SESSION:UUID`), by a detection-time range, or both,
  optionally narrowed by rule. It refuses when the selection exceeds `--max-findings`,
  when the note itself trips the detector, and when nothing selects. Re-running it adds
  nothing.
- The findings summary and the health check leave acknowledged findings out of every
  count that pages, and report them apart as `acked_by_rule`.
- `?days=N` (1 to 90) on the summary and the health check adds `daily`: per UTC day,
  oldest first, every day present, the distinct messages per rule (unacknowledged,
  request copies excluded), as `messages_by_rule` counts them.

## Impact

- Hub binary and one migration; a release and a hub swap via infra. `hub.toml` is unchanged.
- Rollback: the previous binary refuses to start while migration 14 is recorded, so a
  rollback restores the binary and runs `DELETE FROM _sqlx_migrations WHERE version = 14`
  (the table may stay; the old binary does not read it).
- `DETECTOR_VERSION` stays 1.
- No HTTP write route: acknowledging is an operator action on the host that holds the
  database credentials, like `findings redact`. A network route would need a new kind of
  token for a write the hub has never exposed.
