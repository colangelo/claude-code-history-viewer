## Context

Facts this design rests on (repo read 2026-11-02; file:line in the research notes of the change):

- **The analytics mirror.** It is a DuckDB copy of `messages`, `sessions`, `projects`, `message_tool_uses` and `message_tool_results` (`mirror.rs:971-1048`).
  - `conversational` is `role IS NOT NULL`, not #41's `content IS NOT NULL`.
  - The mirror has no host, no message `type` or subtype, and no content.
  - Local days come from `tz_spans` (`stats.rs:148-164`).
- **Client scope.**
  - Lab statistics count only `none` sessions under enforce (R2-E, `mirror.rs:1051`, `visibility.rs:51-61`), and observe filters nothing.
  - No route aggregates client sessions into one bucket today.
  - The route walk (`client_scope_routes_test.rs`) requires a case for every GET route, and a differential test compares lab answers with client rows deleted.
- **Compactions:** Claude Code records `type: system, subtype: compact_boundary`. Codex `compacted` becomes the same (`codex.rs:447-456`).
- **Peer bells** arrive as `isMeta` user rows and are dropped at parse (`claude.rs:1403-1406`). `origin`, `promptId` and `isCompactSummary` are not modelled.
- **Codex MCP calls:** a `custom_tool_call` named `exec`, then `event_msg item_completed` with `item.type = McpToolCall`, `server`, `tool`. The parser drops the latter.
  - `message_key` hashes the message's index and its content (`convert.rs:48-59`).
  - So inserting a message, or changing an archived message's content, re-keys rows and stores duplicates.

## Decisions

### D1. Mirror columns, not live Postgres scans, for `/v1/stats/daily`

**Three new mirror columns.** All three are computed at mirror sync from the Postgres row:
- `has_content = content IS NOT NULL`;
- `prompt = role = 'user' AND has_content AND NOT (every content block is a tool_result)`;
- `compaction = raw->>'subtype' IN ('compact_boundary')`.

They need one rebuild (derived state, about 8 minutes of 503 on the stats routes, as documented).

**Why not Postgres:** a per-day, per-project scan over 2M+ rows on pg1 for every call is the shape #24 measured at 13.7 s.

### D2. Column definitions (monitoring's names and types)

| column | definition |
|---|---|
| `day` | local date in `tz` (default `Europe/Rome`), from `ts_local` |
| `project` | the project identity's display name; `aruba` for the client row (D3) |
| `space` | `ac`, or `aruba` for the client row |
| `sessions` | distinct sessions with any row that day |
| `messages` | `has_content` rows (#41's conversation items). NOT `/v1/stats/global`'s `message_count`, which counts rows with a role. |
| `turns` | `prompt` rows minus `compaction` rows of the same session and day. A compaction is followed by one summary prompt, which is not a turn. |
| `tokens_in` / `tokens_out` | `usage_row` sums, as `/v1/stats/global` |
| `active_seconds` | per session, the sum of gaps between consecutive rows, each gap capped at 300 s, summed per day |
| `ac_messages` | `null` (not derivable from what is stored; see the proposal) |
| `compactions` | `compaction` rows |
| `cost_usd` | sum of reported cost; `null` when no row that day reported one |

### D3. Client sessions: RULED (a) (security, 2026-11-02 18:33Z)

Option (a), on both routes, in observe and enforce. Security's conditions:
1. The client filter sits in the query path the R2-E differential test covers. That test is extended to both routes: a client session with a marker changes no count a lab token sees.
2. No caller parameter filters on content, a project name or a session id. The only parameters are `from`, `to`, `tz` and the tool `prefix`. The 31-day bound holds on BOTH routes, so the counts cannot become an oracle for a client term. Monitoring's history is loaded window by window.
3. Reader tokens `kv/agents/cchv-readers/monitoring` and `.../quota-tap` are scoped to the stats routes only: they read no messages.
4. The #102 mapping is new rows only, and the mirror rebuild's 503 window is announced to monitoring.

The options as written before the ruling:

- **(a), chosen:** A lab reader gets lab sessions only, under enforce and observe alike. Client sessions are absent; no `aruba` row. This is today's R2-E rule applied in observe too, so nothing new needs approval. The differential test then covers the route as it stands.
- **(b)** A counts-only row per day, `project = space = 'aruba'`, built by the hub from `client_class`, never from a name. It is served only to a principal with a new scope `stats-aggregate`, granted per reader token by security. The differential test gets an explicit exception for that one row, and a test proves the row carries no field but counts.



### D4. `/v1/stats/tool-calls?prefix=<name>&since=<RFC3339>`

The answer is `{since, last_ingest, by_tool: [{tool, server, count}], by_host: [{host, count}]}`.
- **Where it reads:** Postgres (`message_tool_uses` ⋈ `messages` ⋈ `machines`), bounded by `since` (at most 31 days; the same bound on `/v1/stats/daily`) and an index on `messages."timestamp"`. The mirror has no host.
- **Matching:** the name's last `__` segment equals `prefix`, or `prefix` is a full name.
- **Client sessions:** lab sessions only (D3).

### D5. #102 without re-keying history

The Codex parser keeps `exec` as today.
- **The link:** when an `item_completed` `McpToolCall` follows, it records `{server, tool}` on that exec message, in a list `mcp` (one `exec` can make several calls). It lands on the most recent tool call only when that call is `exec`. The serialized record carries it in `raw`, and it stays out of `message_key`, like `cwd` and the attachment payload (a pinned test).
- **The hub:** at ingest, an `exec` tool use whose message carries `raw.mcp` becomes one `message_tool_uses` row per call, named `mcp__<server>__<tool>`. The analytics backfill carries `raw.mcp` so it derives the same rows.
- **Scope:** new rows only. Archived rows keep `exec`, by design.

### D6. Tokens and the route walk

- **Tokens:** both routes take lab reader tokens. As built, the stats-only ones live in their own folder, `kv/agents/cchv-stats-readers/<name>`, which the launcher renders as `[[readers]]` with `stats_only = true`: the folder decides the scope, so no flag inside a leaf can be forgotten. A plain reader (`kv/agents/cchv-readers/<name>`) reads the routes too.
- **Writes:** a `read_only` reader cannot write.
- **The route walk:** both routes get cases, with a populated mirror for `daily`.

## Risks

- **`turns` is an approximation:** a session compacted on one day with its summary on the next miscounts by one.
- **The mirror rebuild** is a window of 503 on all stats routes. Infra plans it with the swap.
- **Codex Exa counts start at the release;** earlier Codex calls stay invisible.
