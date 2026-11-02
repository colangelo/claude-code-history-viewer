## Why

Two agents need counts from the archive that no route serves today:

- **monitoring** builds ac's agent dashboard (a1101-41). It wants one row per day and project: sessions, messages, turns, tokens, active time, compactions, cost. `/v1/stats/global` gives per-day totals for all projects together only, and its `top_projects` list can name client projects while client scope is in observe mode.
- **infra's quota-tap** counts Exa calls against a quota. Claude Code's calls are in `message_tool_uses`. Codex's MCP calls are recorded as an `exec` custom tool call plus an `event_msg item_completed` `McpToolCall`, which the parser drops (`codex.rs:1349`), so they are invisible (#102).

Both are counts-only reads behind a reader token. Security reviews them once, together, including how client sessions are counted.

## What changes

- **`GET /v1/stats/daily?from&to&tz`:** one row per local day and project identity, in monitoring's column names: `day, project, space, sessions, messages, turns, tokens_in, tokens_out, active_seconds, ac_messages, compactions, cost_usd`. Client sessions follow security's ruling (design D3): either they are left out, or they are collapsed by the hub into one counts-only row per day with `project = space = 'aruba'`.
- **`GET /v1/stats/tool-calls?prefix&since`:** counts per tool name and per host since a time, plus `last_ingest`. The tool name matches on its last `__` segment, so `mcp__exa__web_search_exa` and a Codex Exa call both count as `web_search_exa`, and the server is kept in the answer.
- **The mirror gains three per-message columns:** `has_content` (#41's conversation item), `prompt` (a user turn that is not only tool results) and `compaction` (a compaction boundary record). This takes one mirror rebuild, the documented ~8 minutes of 503 on stats routes.
- **#102, the Codex parser:** an `McpToolCall` completion is attached to its `exec` call as `mcp: {server, tool}` in the record's `raw`, outside the dedup key. The hub then records the tool use as `mcp__<server>__<tool>`. New rows only; history is not backfilled.
- **Reader tokens:** `kv/agents/cchv-readers/monitoring` and `kv/agents/cchv-readers/quota-tap`, minted by infra after the release. They are lab readers.

## Not in this change

- `ac_messages` ships as `null`. Nothing stored tells a prompt ac typed from other user turns: Claude Code's `origin` is not modelled. Peer bells cannot inflate it, because they are `isMeta` and never archived. Storing `origin` is a daemon follow-up.
- Who runs monitoring's loader (manager-lab's call; infra's launchd job on m4m is the proposal).
- `/v1/stats/global` is unchanged; it is a contract.
