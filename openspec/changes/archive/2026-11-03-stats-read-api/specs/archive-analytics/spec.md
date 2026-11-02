## ADDED Requirements

### Requirement: Daily stats per project

The hub SHALL serve `GET /v1/stats/daily?from&to&tz` to an authenticated reader. It answers one row per local day and project identity, with the columns `day, project, space, sessions, messages, turns, tokens_in, tokens_out, active_seconds, ac_messages, compactions, cost_usd`, defined in design D2. A lab reader SHALL see lab sessions only, in observe and enforce alike, and `from`/`to` SHALL span at most 31 days. The only parameters SHALL be `from`, `to` and `tz`.

#### Scenario: A compaction is not a turn
- **WHEN** a session has, on one day, two user prompts, one compaction boundary and the summary prompt that follows it
- **THEN** that day's row counts `turns = 2` and `compactions = 1`

#### Scenario: A day without reported cost
- **WHEN** no row of a project on a day carries a cost
- **THEN** `cost_usd` is null for that row, not 0

#### Scenario: A client session changes nothing a lab reader sees
- **WHEN** a client-class session with a marker is added to a window
- **THEN** every count a lab reader gets for that window is unchanged

### Requirement: Tool-call counts

The hub SHALL serve `GET /v1/stats/tool-calls?prefix&since` to an authenticated reader. It answers counts per tool name (with its MCP server) and per host since `since`, at most 31 days back, and the time of the newest message the reader may see. A tool name SHALL match when its last `__` segment equals `prefix`.

#### Scenario: Claude and Codex Exa calls count together
- **WHEN** one Claude session calls `mcp__exa__web_search_exa` and one Codex session completes an Exa `McpToolCall` after this change
- **THEN** `prefix=web_search_exa` counts 2, with server `exa`

#### Scenario: A window past the bound is refused
- **WHEN** `since` is more than 31 days back
- **THEN** the hub answers 400

### Requirement: Codex MCP calls keep their dedup key

The Codex parser SHALL record a completed `McpToolCall` as `mcp: {server, tool}` on its `exec` message, outside `message_key`. An archived row SHALL NOT be stored twice because of this change.

#### Scenario: Re-syncing an archived rollout
- **WHEN** a rollout synced before this change is read again by a daemon with this change
- **THEN** every message keeps its `message_key`, and the hub stores no new row for it
