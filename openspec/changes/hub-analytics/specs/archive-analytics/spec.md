## ADDED Requirements

### Requirement: Usage totals deduplicated per provider message

Every token, cost, and message-count rollup SHALL count a provider message's
`usage` block exactly once. A single assistant response recurs across multiple
stored rows carrying the same `usage` values, so aggregates MUST deduplicate on
`(session_id, message_id)` — falling back to `(session_id, uuid)` when
`message_id` is absent, and counting the row unconditionally when both are
absent — before summing. A plain `SUM` over `messages` is not a conforming
implementation.

This mirrors `dedup_token_totals` in the retired desktop implementation
(`src-tauri/src/commands/stats.rs`), which is the verification oracle for this
capability.

#### Scenario: Repeated usage block is counted once

- **WHEN** a session contains multiple message rows sharing one `message_id` and an identical `usage` block
- **THEN** the session's token totals include that block's tokens exactly once

#### Scenario: Messages without a provider message id fall back to uuid

- **WHEN** a session contains rows with no `message_id` but distinct `uuid` values
- **THEN** each distinct `uuid` contributes its usage exactly once

#### Scenario: Totals agree with the desktop oracle

- **WHEN** global, project, and session statistics are computed over a corpus that the desktop implementation has also analyzed
- **THEN** the token totals, message counts, and cost totals from the hub match the desktop results for the same scope and date window

### Requirement: Global statistics endpoint

The hub SHALL expose `GET /v1/stats/global` returning archive-wide aggregate
statistics as a `GlobalStatsSummary`: totals across projects, sessions, and
messages; token and cost totals; per-model and per-provider breakdowns; and
daily activity. The endpoint MUST accept optional inclusive `from` and `to`
date-window parameters, and MUST require a valid read token.

#### Scenario: Authenticated request returns archive-wide totals

- **WHEN** an authenticated client GETs `/v1/stats/global`
- **THEN** the hub responds `200` with token totals, cost totals, per-model and per-provider breakdowns, and daily activity spanning the archive

#### Scenario: Date window restricts the aggregate

- **WHEN** an authenticated client GETs `/v1/stats/global` with `from` and `to`
- **THEN** only messages whose timestamp falls inside the inclusive window contribute to the response

#### Scenario: Unauthenticated request is rejected

- **WHEN** a client GETs `/v1/stats/global` without a valid read token
- **THEN** the hub responds `401` and returns no statistics

### Requirement: Per-project statistics endpoint

The hub SHALL expose `GET /v1/stats/projects/{identity_key}` returning a
`ProjectStatsSummary` for one project identity: session and message counts,
token totals, average and total session duration, most active hour, and most
used tools. Statistics MUST be folded across every path and machine belonging to
that identity, so a repository that was moved, cloned, or checked out as a
worktree reports as one project.

#### Scenario: Statistics fold across machines and paths

- **WHEN** a project identity spans several `project_path` values on more than one machine
- **THEN** the response aggregates sessions and messages from all of them under the single identity

#### Scenario: Unknown identity returns not found

- **WHEN** an authenticated client requests statistics for an identity key that does not exist
- **THEN** the hub responds `404`

### Requirement: Per-session statistics endpoint

The hub SHALL expose `GET /v1/stats/sessions/{id}` returning a
`SessionTokenStats` for one session: the four token counts plus reasoning
tokens, total tokens, message count, first and last message time, the session
summary when present, and its most used tools.

#### Scenario: Session statistics are returned for a known session

- **WHEN** an authenticated client requests statistics for an existing session id
- **THEN** the hub responds `200` with that session's token counts, message count, and time bounds

#### Scenario: Unknown session returns not found

- **WHEN** an authenticated client requests statistics for a session id that does not exist
- **THEN** the hub responds `404`

### Requirement: Tool and skill usage statistics

Statistics responses SHALL report tool usage as `ToolUsageStats` — tool name,
invocation count, and success rate — computed from the stored tool-invocation
records rather than by scanning message JSONB at query time. Claude `Skill`
invocations MUST additionally be reported keyed by skill name, and Claude
`Agent` invocations keyed by subagent type, so each is distinguishable from the
generic tool that carries it. Both collections are empty for providers that
expose no such abstraction.

#### Scenario: Tool counts and success rates are reported

- **WHEN** a scope contains tool invocations of which some are errors
- **THEN** the response lists each tool with its invocation count and a success rate reflecting the non-error proportion

#### Scenario: Skills are reported by name

- **WHEN** a scope contains `Skill` tool invocations naming different skills
- **THEN** the response reports usage per skill name, not a single aggregated `Skill` entry

#### Scenario: Subagents are reported by type

- **WHEN** a scope contains `Agent` tool invocations naming different subagent types
- **THEN** the response reports usage per subagent type, separately from tools and skills

### Requirement: Activity rhythm statistics

Statistics responses SHALL include activity distributed over time as
`DailyStats` (per-day tokens, message count, session count, and active hours)
and `ActivityHeatmap` (hour-of-day by day-of-week activity counts and tokens),
computed in a caller-supplied timezone so local working hours are meaningful.

#### Scenario: Daily activity is bucketed per day

- **WHEN** a scope spans several days
- **THEN** the response contains one daily entry per day with activity, carrying that day's tokens, message count, and session count

#### Scenario: Heatmap buckets by local hour and weekday

- **WHEN** activity statistics are requested with a timezone
- **THEN** each message contributes to the hour-of-day and day-of-week bucket corresponding to its timestamp in that timezone
