## 1. Review

- [ ] 1.1 Second review of this proposal by the client-side security reviewer (Claude-only, client work), via the client's manager
- [ ] 1.2 manager-lab's go to implement, after 1.1

## 2. Hub

- [ ] 2.1 `browse.rs` messages query: `m.raw->>'cwd' AS cwd`, `MessageRow.cwd: Option<String>`
- [ ] 2.2 Test: a message ingested with a `cwd` returns it; one without returns null

## 3. Distiller

- [ ] 3.1 `build_transcript` returns a routing view (cwds, full tool-input text, full message text) from the same reads
- [ ] 3.2 `Routing.is_protected(project, prompt, view)`: markers on path, cwd and tool input; terms on full text and prompt
- [ ] 3.3 Fail closed: a hub read error protects the group, logged
- [ ] 3.4 Log the matched input class only, never the marker, term or text
- [ ] 3.5 Tests, each with a must-not-protect control: cwd under a marker; tool input path under a marker; a term only in the cut middle of a long session; a term only in a tool input past 300 chars; a hub read error; an older hub without `cwd`

## 4. Release

- [ ] 4.1 Gate, release, hub swap before the distiller reinstall (deploy order), on ac's word
- [ ] 4.2 Read back: one night's route counts by class, sent to the client's manager and its security reviewer
