## 0. Before building

- [ ] 0.1 Security reviews this change, and rules on D3: (a) lab rows only, or (b) a counts-only `aruba` row under a `stats-aggregate` scope.
- [ ] 0.2 monitoring confirms the column definitions (D2) for its panel labels.

## 1. Mirror

- [ ] 1.1 Red: a mirror test with a compaction boundary, a compact-summary prompt, a tool-result-only user row and a mixed user row expects `has_content`, `prompt` and `compaction` values.
- [ ] 1.2 Add the three columns at mirror sync, with a schema-version bump that forces one rebuild.

## 2. `GET /v1/stats/daily`

- [ ] 2.1 Red: route tests over a populated mirror, one per D2 column. These include a day with no cost (null), the 300 s gap cap, and a compaction that does not count as a turn.
- [ ] 2.2 The route, with D3 as ruled. Route-walk case. Differential test (or its documented exception under b).

## 3. `GET /v1/stats/tool-calls`

- [ ] 3.1 Red: counts per tool and host since T. A last-segment match. The 31-day bound is refused past it. Client sessions follow D3.
- [ ] 3.2 The route, plus an index migration on `messages ("timestamp")` if the plan needs it (EXPLAIN on a pg1 copy first). Route-walk case.

## 4. #102 Codex MCP calls

- [ ] 4.1 Red: a rollout with an `exec` call and an `McpToolCall` completion yields `raw.mcp = {server, tool}` on the exec message, with an unchanged `message_key` (a pinned test). The hub records `mcp__exa__web_search_exa`.
- [ ] 4.2 Parser and ingest mapping. New rows only.

## 5. Ship

- [ ] 5.1 Docs: `deployment.md` (routes, tokens, the rebuild window), and monitoring's definitions.
- [ ] 5.2 Gate, security's code review, release under the standing go. Infra mints the `monitoring` and `quota-tap` reader tokens and plans the mirror rebuild with the swap.
