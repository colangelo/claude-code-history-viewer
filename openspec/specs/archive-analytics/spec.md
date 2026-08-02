# archive-analytics Specification

## Purpose

Archive-wide analytics served by the hub: aggregate statistics over every
machine's ingested history, scoped globally, per project identity, or per
session. Defines what the statistics endpoints return and the counting rules
they must obey — chiefly that a provider message's usage is counted exactly
once no matter how many rows store it.

## Requirements

### Requirement: Usage totals deduplicated per provider message

Token and cost totals SHALL count each provider message once, even though one
assistant response is stored across several rows sharing a provider message id
and repeating an identical usage block. Deduplication SHALL identify a logical
message by its provider message id where present, falling back to the record
uuid and then the row id, and SHALL attribute usage to exactly one row of each
logical message.

Deduplication MUST NOT be applied to tool invocations: the rows sharing a usage
block carry different content blocks, so their tool calls are distinct events.

The usage-bearing row of a logical message SHALL be determined over the archive
as a whole rather than recomputed per requested window. A logical message whose
rows straddle a window boundary MAY therefore contribute its usage to neither
side of that boundary; this is bounded and negligible, and is preferred to
recomputing deduplication on every request.

#### Scenario: Repeated usage blocks count once

- **WHEN** several stored rows share one provider message id and repeat its usage block
- **THEN** the totals count that message's tokens exactly once

#### Scenario: Rows without a provider message id are not collapsed

- **WHEN** stored rows carry distinct record uuids and no provider message id
- **THEN** each contributes its own usage

#### Scenario: Tool counts are not deduplicated

- **WHEN** rows sharing a provider message id each carry distinct tool invocations
- **THEN** every invocation is counted

### Requirement: Global statistics endpoint

The hub SHALL expose `GET /v1/stats/global` returning archive-wide aggregate
statistics as a `GlobalStatsSummary`: totals across projects, sessions, and
messages; token and cost totals; per-model and per-provider breakdowns; and
daily activity. The endpoint MUST accept optional inclusive `from` and `to`
date-window parameters, and MUST require a valid read token.

Responses SHALL be computed from the derived statistics mirror. The endpoint
SHALL be interactive: an unwindowed archive-wide request MUST complete in
well under a second on a mirror of the current archive's scale, rather than
scaling with the number of stored messages on every call.

#### Scenario: Authenticated request returns archive-wide totals

- **WHEN** an authenticated client GETs `/v1/stats/global`
- **THEN** the hub responds `200` with token totals, cost totals, per-model and per-provider breakdowns, and daily activity spanning the archive

#### Scenario: Date window restricts the aggregate

- **WHEN** an authenticated client GETs `/v1/stats/global` with `from` and `to`
- **THEN** only messages whose timestamp falls inside the inclusive window contribute to the response

#### Scenario: The default view is interactive

- **WHEN** an authenticated client requests the archive-wide statistics the webapp loads by default
- **THEN** the response completes in well under a second

#### Scenario: Unauthenticated request is rejected

- **WHEN** a client GETs `/v1/stats/global` without a valid read token
- **THEN** the hub responds `401` and returns no statistics

### Requirement: Per-project statistics endpoint

The hub SHALL expose `GET /v1/stats/projects/{identity_key}` returning a
`ProjectStatsSummary` for one project identity: session and message counts,
token totals, average and total session duration, most active hour, most used
tools, and a per-model usage distribution (`model_distribution`) with the same
shape and dedup semantics as the global endpoint's. Statistics MUST be folded
across every path and machine belonging to that identity, so a repository that
was moved, cloned, or checked out as a worktree reports as one project.

#### Scenario: Statistics fold across machines and paths

- **WHEN** a project identity spans several `project_path` values on more than one machine
- **THEN** the response aggregates sessions and messages from all of them under the single identity

#### Scenario: Unknown identity returns not found

- **WHEN** an authenticated client requests statistics for an identity key that does not exist
- **THEN** the hub responds `404`

#### Scenario: Per-model distribution at project scope

- **WHEN** an authenticated client requests statistics for a known identity whose messages carry model names
- **THEN** the response includes `model_distribution` entries with per-model message and token counts (input, output, cache-creation, cache-read) covering only that identity's messages, within any requested date window

### Requirement: Per-session statistics endpoint

The hub SHALL expose `GET /v1/stats/sessions/{id}` returning a
`SessionTokenStats` for one session: the four token counts plus reasoning
tokens, total tokens, message count, first and last message time, the session
summary when present, and its most used tools.

The `{id}` segment SHALL accept either the session's archive row id or its
provider session UUID, resolved by the same rule as
`GET /v1/sessions/{id}/messages`. A caller MUST NOT have to know which of the two
identifiers a given route expects: a UUID that identifies no session SHALL be
reported as not found rather than as a malformed request.

#### Scenario: Session statistics are returned for a known session

- **WHEN** an authenticated client requests statistics for an existing session id
- **THEN** the hub responds `200` with that session's token counts, message count, and time bounds

#### Scenario: A session UUID resolves the same as a row id

- **WHEN** an authenticated client requests statistics using a session's UUID
- **THEN** the hub responds `200` with the same statistics as the equivalent row-id request

#### Scenario: Unknown session returns not found

- **WHEN** an authenticated client requests statistics for a session id that does not exist, in either accepted form
- **THEN** the hub responds `404`

### Requirement: Tool and skill usage statistics

Statistics responses SHALL report tool usage as `ToolUsageStats` — tool name,
invocation count, and success rate — computed from the stored tool-invocation
records rather than by scanning message JSONB at query time. Claude `Skill`
invocations MUST additionally be reported keyed by skill name, and Claude
`Agent` invocations keyed by subagent type, so each is distinguishable from the
generic tool that carries it. Both collections are empty for providers that
expose no such abstraction.

Success rate MUST be derived by resolving each invocation against the outcome
that reports on it, preferring an outcome carried by a later message over one
carried on the invocation's own record, and treating an invocation with no
recorded outcome as successful.

#### Scenario: Tool counts and success rates are reported

- **WHEN** a scope contains tool invocations of which some are errors
- **THEN** the response lists each tool with its invocation count and a success rate reflecting the non-error proportion

#### Scenario: An errored outcome in a later message lowers the success rate

- **WHEN** an invocation's error outcome is reported by a different, later message
- **THEN** that invocation counts as a failure in the tool's success rate

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

### Requirement: Cost is a first-class analytics metric

The Analytics view SHALL present estimated cost as the first headline metric at
both the whole-archive and single-project scopes, derived by pricing the
scope's `model_distribution` with the client-side pricing table, and SHALL
display the pricing-coverage percentage alongside it. Where a figure's grain
has no per-model split (top projects, provider distribution, daily trend), the
view MUST NOT display an invented cost for it.

#### Scenario: Project scope shows cost

- **WHEN** a user selects a single project identity in the Analytics scope control
- **THEN** an estimated cost figure for that project is displayed, priced from the project's `model_distribution`

#### Scenario: Unpriceable grains carry no cost

- **WHEN** the view renders rows whose source data has no per-model split
- **THEN** those rows show token totals without a cost figure

### Requirement: Analytics range control offers presets and custom windows

The Analytics view SHALL offer relative date-window presets of 7, 14, 30, 60,
90, 180, and 365 days plus all time, and SHALL accept a custom "last N days"
positive integer. The selected window MUST apply to both scopes.

#### Scenario: Preset selection narrows the window

- **WHEN** a user picks the 7-day preset
- **THEN** the statistics request covers the last 7 days

#### Scenario: Custom day count

- **WHEN** a user enters 45 as a custom day count
- **THEN** the statistics request covers the last 45 days

### Requirement: Activity heatmap precedes tool usage

In both the whole-archive and single-project analytics layouts, the activity
heatmap SHALL appear in the first chart row, before tool-usage charts.

#### Scenario: Heatmap in the first chart row

- **WHEN** the Analytics view renders either scope with activity data present
- **THEN** the activity heatmap is in the first row of chart sections and the most-used-tools chart appears after it

