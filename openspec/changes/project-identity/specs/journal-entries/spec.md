# journal-entries Delta

## ADDED Requirements

### Requirement: Identity-scoped journal reads

`GET /v1/journal/entries` and journal search SHALL accept the
`identity:<key>` project filter (expansion to member + aliased paths,
`include_worktrees` honored), so a moved repo's journal timeline reads as one
stream. Entry storage stays keyed by `(entry_date, project_path)` — identity
is a read-time lens; the pending work-list contract, write endpoint, and
distiller are unchanged. When a logical day has entries under two member
paths (a mid-day move), both entries are returned and the client renders them
under one identity heading.

#### Scenario: Unified timeline across a move

- **WHEN** a repo moved homes on day D and journal entries exist for the old path (before D) and new path (after D)
- **THEN** `GET /v1/journal/entries?project=identity:<key>` returns the full timeline across both paths

#### Scenario: Distiller contract untouched

- **WHEN** the distiller polls `/v1/journal/pending` and posts entries after this change ships
- **THEN** its requests and the hub's responses are identical to pre-identity behavior
