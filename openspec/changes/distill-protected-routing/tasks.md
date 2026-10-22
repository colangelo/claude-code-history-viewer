## 1. Distiller

- [x] 1.1 Load `distill-routing.toml` (`CCHV_DISTILL_ROUTING` override); missing or invalid → protect all.
- [x] 1.2 Route a group as protected on a path marker or a content term in the prompt.
- [x] 1.3 Protected call: `claude -p --model <exact id>`, with `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` stripped.
- [x] 1.4 Log the serving model of every call; refuse and do not store a protected result unless `modelUsage` is exactly the id.
- [x] 1.5 `journal_entries.model` = the serving model.
- [x] 1.6 Tests: path route, content route, general route, fail-closed config, the guard (exact, other model, extra model, missing).

## 2. Release and ops

- [x] 2.1 cchv-v0.27.0 tagged and published; install only the distiller (hub and webapp unchanged).
- [x] 2.2 Write the routing file on m4m before installing.
- [x] 2.3 Remove the interim plist overrides (done 2026-10-21 11:17Z; the synthetic two-route probe passed); prove one protected group served by the protected id and one general group served by GPT, from the call log. Real run 2026-10-22 04:19Z: 27 groups, 0 failed, 0 refused; every protected call served claude-haiku-5-5, general calls gpt-6.1-sol; no protected path stored a GPT row.
