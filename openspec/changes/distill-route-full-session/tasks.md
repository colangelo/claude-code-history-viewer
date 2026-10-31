## 1. Review

- [x] 1.1 Second review of revision 1 by the client-side security reviewer (two separate Claude reviews, merged as M1–M10, 2026-10-31): revise, then go
- [ ] 1.2 Re-test of revision 2 against M1–M4 (one Sonnet 5.5 high session, client side)
- [ ] 1.3 manager-lab's go to implement, after 1.2

## 2. Working folder in the stored record (M1)

- [ ] 2.1 `history-core`: `ClaudeMessage.cwd: Option<String>`, filled by the Claude provider from the source record
- [ ] 2.2 Daemon test through the real convert path: a source line with `cwd` stores it in `raw`; one without stores none
- [ ] 2.3 Hub: messages query returns `m.raw->>'cwd' AS cwd`; test that a message ingested with `cwd` returns it and one without returns null
- [ ] 2.4 End to end: a Claude Code JSONL with a marker-bearing `cwd` and a neutral project path → ingest → messages endpoint → group protected; neutral `cwd` control → general

## 3. Distiller routing

- [ ] 3.1 One `flatten` producing the capped transcript line and the uncapped view parts; property test over every block type: each capped line ⊆ the view text (M4)
- [ ] 3.2 Routing view reads each session unwindowed, sidechains included; transcript keeps the day window and skips sidechains (M5, M6)
- [ ] 3.3 Matcher: markers on path, cwd, tool-input values and full text; terms on full text; normalization of URL-encoding, backslashes, Glob/Grep path+pattern, marker as a path segment (M2, M8)
- [ ] 3.4 Send-time gate on the general route: no marker or term in the exact prompt, else protect (class `prompt`) (M4)
- [ ] 3.5 Fail closed with classes: unreadable or partial session, unknown content shape (next group continues), `cwd` key absent from an updated hub, routing file below floor (M9)
- [ ] 3.6 Log the class only; per-class counts at the end of a run
- [ ] 3.7 Tests, each with a must-not-protect twin: marker only in a user prompt, a compaction summary, a tool result, a peer message, a sidechain tool input, a later day of the same session, an offered thread; each M8 path shape; a term only in a cut middle; a hub stub without `cwd`; N = 2 sessions with one failing read

## 4. Routing file and gates (M3, M7)

- [ ] 4.1 Routing file, by the client's manager: the terms and markers #10 needs (interim set applied 2026-10-31 15:12Z, fingerprint recorded with the client's manager)
- [ ] 4.2 #13's floor and fingerprint: the distiller checks counts and fingerprint at start, else protects everything
- [ ] 4.3 #12's origin filter ships with or before this change (release gate)
- [ ] 4.4 #14's hardened protected call: land with this change if ready (recommended by both reviewers); #18 linked

## 5. Release

- [ ] 5.1 Gate, release; hub swap, then the daemon on both Macs, then the distiller reinstall; on ac's word
- [ ] 5.2 Read back one night: route counts by class; general-routed groups whose full text holds any marker or term = 0; sent to the client's manager and its security reviewer
- [ ] 5.3 #10 closes only when 2–4 and the routing-file half (4.1, 4.2) are in
