## 0. Before building

- [x] 0.1 Security reviews this change and rules on D3: (a), with conditions 1-4 (design D3).
- [x] 0.2 monitoring confirms the column definitions (D2) for its panel labels (bd5b8fad, 18:55Z; no column change; ac_messages not drawn).

## 1. Mirror

- [ ] 1.1 Red: a mirror test with a compaction boundary, a compact-summary prompt, a tool-result-only user row and a mixed user row expects `has_content`, `prompt` and `compaction` values.
- [ ] 1.2 Add the three columns at mirror sync, with a schema-version bump that forces one rebuild.

## 2. `GET /v1/stats/daily`

- [ ] 2.1 Red: route tests over a populated mirror, one per D2 column. These include a day with no cost (null), the 300 s gap cap, and a compaction that does not count as a turn.
- [ ] 2.2 The route, with lab sessions only (D3 a) and the 31-day bound. Route-walk case. The R2-E differential test is extended to this route.

## 3. `GET /v1/stats/tool-calls`

- [ ] 3.1 Red: counts per tool and host since T. A last-segment match. The 31-day bound is refused past it. Lab sessions only (D3 a). The R2-E differential test is extended to this route.
- [ ] 3.2 The route, plus an index migration on `messages ("timestamp")` if the plan needs it (EXPLAIN on a pg1 copy first). Route-walk case.

## 3b. Stats-only reader tokens (D3 condition 3)

- [ ] 3b.1 Red: a reader marked `stats_only` gets 403 on every non-stats GET route (route-walk driven) and 200 on both stats routes.
- [ ] 3b.2 The `stats_only` reader flag in config and in the launcher's `[[readers]]` rendering.

## 4. #102 Codex MCP calls

- [ ] 4.1 Red: a rollout with an `exec` call and an `McpToolCall` completion yields `raw.mcp = {server, tool}` on the exec message, with an unchanged `message_key` (a pinned test). The hub records `mcp__exa__web_search_exa`.
- [ ] 4.2 Parser and ingest mapping. New rows only.

## 5. Ship

- [ ] 5.1 Docs: `deployment.md` (routes, tokens, the rebuild window), and monitoring's definitions.
- [ ] 5.2 Gate, security's code review, release under the standing go. Infra mints the `monitoring` and `quota-tap` reader tokens and plans the mirror rebuild with the swap.
