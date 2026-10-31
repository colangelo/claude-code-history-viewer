## 1. Review

- [x] 1.1 Second review of revision 1 by the client-side security reviewer (two separate Claude reviews, merged as M1–M10, 2026-10-31): revise, then go
- [x] 1.2 Re-test of revision 2 against M1–M4 (one Sonnet 5.5 high session, client side): go with conditions C1–C8; C1–C4 in this revision (3)
- [x] 1.3 manager-lab's go to implement, after 1.2

## 2. Working folder in the stored record (M1)

- [x] 2.1 `history-core`: `ClaudeMessage.cwd: Option<String>`, filled by the Claude provider from the source record
- [x] 2.2 Daemon test through the real convert path: a source line with `cwd` stores it in `raw`; one without stores none
- [x] 2.3 Hub: messages query returns `m.raw->>'cwd' AS cwd`; test that a message ingested with `cwd` returns it and one without returns null
- [x] 2.4 End to end: a Claude Code JSONL with a marker-bearing `cwd` and a neutral project path → ingest → messages endpoint → group protected; neutral `cwd` control → general (daemon e2e test JSONL → real daemon → real hub → messages endpoint; the distiller half routes that shape in `test_a_cwd_under_a_marker_protects`)

## 3. Distiller routing

- [x] 3.1 One `flatten` producing the capped transcript line and the uncapped view parts; property test over every block type: each capped line ⊆ the view text (M4)
- [x] 3.2 Routing view reads each session unwindowed, sidechains included; transcript keeps the day window and skips sidechains (M5, M6)
- [x] 3.3 Matcher: markers on path, cwd, tool-input values and full text; terms on full text; normalization of URL-encoding, backslashes, Glob/Grep path+pattern, marker as a path segment (M2, M8)
- [x] 3.4 Send-time gate wrapping the single send function: every outbound string of a general call (system and user text, every retry, both general backends), normalized as the view is; a hit refuses the send and protects (class `prompt`); test with a stub that records all outbound strings (M4, C4)
- [x] 3.5 Fail closed with classes: unreadable or partial session, a group over the read budget, unknown content shape (next group continues), a message with no `cwd` key (protect, `cwd_unavailable`; `null` is no signal), routing file below floor (M9, C3, C8)
- [x] 3.6 Log the class only; per-class counts at the end of a run
- [x] 3.8 No model or CLI text in any log line or raised message on a protected group: length and hash only (security-aruba #18, review 3 M9); tests: a stub `claude -p` exiting non-zero, emitting non-JSON, returning an error result, a non-JSON entry, an invalid entry quoting its topics, a hub 400 echo; no substring of the stub output longer than 8 characters reaches the log
- [x] 3.7 Tests, each with a must-not-protect twin: marker only in a user prompt, a compaction summary, a tool result, a peer message, a sidechain tool input, a later day of the same session, an offered thread; each M8 path shape; the client folder name followed by a space, a backtick, a quote, a colon, a newline and the end of the string, each with a lab twin whose longer name only starts with it (C2); a term only in a cut middle; a hub stub whose messages have no `cwd` key → protected, and one with `cwd: null` → general (C3); a malformed content item → that group protected, the next group processed (C7); N = 2 sessions with one failing read

## 4. Routing file and gates (M3, M7)

- [x] 4.1 Routing file, entries by kind (C1), by the client's manager's decision, applied by cchv 2026-10-31: plain path markers (the client roots and repos, 11); their encoded storage forms (`/` and `_` → `-`, 10); content terms (6), among them the single client-name term that covers the org URL form, the client's hosts, its controller and reviewer session names and the encoded form in text. Acceptance probe per entry through the INSTALLED loader against the LIVE file: 11/11, 10/10 and 6/6 routed protected; 3 lab controls 0/3. Fingerprint recorded with the client's manager.
- [x] 4.2 #13's floor and fingerprint in `~/.config/cchv/distill-routing.floor.json` (not the routing file), checked at start, else protect everything; `--record-routing-floor` re-records after a widening edit and refuses to lower a count (C6)
- [x] 4.3 #12's origin filter ships with or before this change (release gate), as the change `distill-thread-origin-filter`, with its test: a project with one protected day then one general day → the general prompt offers 0 threads from the protected entry (C5): shipped together in cchv-v0.34.0
- [x] 4.4 #14's hardened protected call: NOT in this release (manager-lab 2026-10-31 16:53Z); the first follow-up after it (Gitea #94)
- [x] 4.5 Review 3 (thread origin) M4: the segment boundary by character class (before: not [a-z0-9_]; after: not [a-z0-9_-]); prose punctuation and wrappers protect, longer names do not (release gate)

## 5. Release

- [x] 5.1 Gate, release; hub swap, then the daemon on both Macs, then the distiller reinstall; on ac's word: released as cchv-v0.34.0 on 2026-10-31, ac gestore-lab.a1031-18; the gate on a672c68d, then hub, daemons and distiller in that order
- [ ] 5.2 Read back one night: route counts by class; general-routed groups whose full text holds any marker or term = 0; sent to the client's manager and its security reviewer
- [ ] 5.3 #10 closes only when 2–4 and the routing-file half (4.1, 4.2) are in
- [x] 5.4 Follow-ups (C8): an issue for Codex and Pi `cwd` (turn level); the daemon-lag window in `docs/residuals.md` until both Macs run the new daemon (Gitea #92; the residual is in `docs/residuals.md`)
