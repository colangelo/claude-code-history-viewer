## Why

`sessions.message_count` is not a count of conversation turns. About half of the
`claude` rows in `messages` have `content IS NULL` — sidecar state records (`attachment`,
`mode`, `permission-mode`, `custom-title`, …). Measured on pg1 2026-10-01: 1,010,831 of
2,039,210 rows (49.6 %); `attachment` alone is 73 % of them. Counting rows as
"messages" therefore overstates the real conversation, and the journal's ~50 % skip rate
reads as a bug to anyone who does not know why. (Issue ac/claude-code-history-viewer#41;
the design proposal there re-measured the premise after #30's cleanup and chose this
additive slice.)

## What Changes

- Hub: `GET /v1/sessions` rows gain `conversation_count` — the rows of that session with
  `content IS NOT NULL`. `message_count` is unchanged; its doc comment now says it is a
  stored counter that can exceed the stored rows (#47), not a row count.
- Webapp: the session row shows `conversation_count` as the headline count, with a
  tooltip saying state records are not counted. New i18n keys in all five locales.
- Docs: one paragraph in `docs/archive/` on `message_count` vs conversation items, and why the
  journal legitimately skips about half of its days.

Not changing: no migration, no daemon change, no rename of `message_count`, no
ingest-time filtering (it would orphan 233,915 `parent_uuid` links for ~7 % of row
bytes), no project-level conversation count (aggregating every session per project is
the expensive shape and nobody asked), and the per-session messages endpoint's
`X-Total-Count`, which drives paging over stored rows.

## Impact

- `crates/hub/src/browse.rs` (`SessionRow`, `list_sessions`), `.sqlx` offline cache
- `src/services/hubApi.ts`, `src/types/archive.ts`, `src/components/ArchiveBrowser/*`,
  `src/i18n/locales/*/session.json`
- `docs/archive/`
- API is additive: existing clients ignore the new field.
