## MODIFIED Requirements

### Requirement: Daily stats per project

The hub SHALL serve `GET /v1/stats/daily?from&to&tz` to an authenticated reader. It answers one row per local day and project identity, with the columns `day, project, space, sessions, messages, turns, tokens_in, tokens_out, active_seconds, ac_messages, compactions, cost_usd`, defined in design D2. A lab reader SHALL see lab sessions only, in observe and enforce alike, and `from`/`to` SHALL span at most 31 days. The only parameters SHALL be `from`, `to` and `tz`. A 200 to a lab reader SHALL be complete: when a lab-eligible session in the window (class `none` or unset) is unscanned under the live generation and its last message is older than the stale margin, the hub SHALL answer 503 instead. Such a session inside the margin SHALL be left out, and the answer SHALL carry their count as `unscanned_excluded`.

#### Scenario: A compaction is not a turn
- **WHEN** a session has, on one day, two user prompts, one compaction boundary and the summary prompt that follows it
- **THEN** that day's row counts `turns = 2` and `compactions = 1`

#### Scenario: A day without reported cost
- **WHEN** no row of a project on a day carries a cost
- **THEN** `cost_usd` is null for that row, not 0

#### Scenario: A client session changes nothing a lab reader sees
- **WHEN** a client-class session with a marker is added to a window
- **THEN** every count a lab reader gets for that window is unchanged

#### Scenario: A generation change without a sweep is stale, never empty
- **WHEN** the hub adopts a new generation and no sweep has run, and the window holds a lab session last written before the margin
- **THEN** a lab reader gets 503 naming the unscanned count and the generation, never a 200 with that session missing

#### Scenario: After the sweep the rows come back
- **WHEN** a sweep stamps every session of that window
- **THEN** the same request answers 200 with the session counted and `unscanned_excluded = 0`

#### Scenario: A session still being written is left out and counted
- **WHEN** the only unscanned lab-eligible session in the window was written inside the margin
- **THEN** the answer is 200 with `unscanned_excluded = 1`

#### Scenario: An unscanned client session does not make the answer stale
- **WHEN** the only unscanned session in the window already carries a client class
- **THEN** the answer is 200 with `unscanned_excluded = 0`

### Requirement: Tool-call counts

The hub SHALL serve `GET /v1/stats/tool-calls?prefix&since` to an authenticated reader. It answers counts per tool name (with its MCP server) and per host since `since`, at most 31 days back, and the time of the newest message the reader may see. A tool name SHALL match when its last `__` segment equals `prefix`. The completeness rule of the daily route SHALL apply over `since` to now, with the same 503 and the same `unscanned_excluded` field.

#### Scenario: Claude and Codex Exa calls count together
- **WHEN** one Claude session calls `mcp__exa__web_search_exa` and one Codex session completes an Exa `McpToolCall` after this change
- **THEN** `prefix=web_search_exa` counts 2, with server `exa`

#### Scenario: A window past the bound is refused
- **WHEN** `since` is more than 31 days back
- **THEN** the hub answers 400

#### Scenario: Tool counts are stale after a generation change
- **WHEN** a lab session since `since`, last written before the margin, is unscanned under the live generation
- **THEN** a lab reader gets 503
