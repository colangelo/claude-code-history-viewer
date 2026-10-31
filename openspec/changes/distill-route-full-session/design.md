## Context

`generate()` calls `routing.is_protected(project, prompt)` on the prompt built from
`build_transcript`, which has already cut tool inputs (300 chars), thinking (500) and each session
to a head and a tail, skips sidechains, and reads only the group's day. The stored message record
(`ClaudeMessage`, serialized by the daemon as `raw`) has no `cwd`; only the parse-side `RawLogEntry`
does, so `m.raw->>'cwd'` is null for every row (measured on pg1 2026-10-31: 0 of the latest 19,857
rows). Review findings M1–M10 refer to the client-side review of revision 1.

## Decisions

1. **Store `cwd` at the source (M1).** `ClaudeMessage.cwd: Option<String>` (serde skip when none),
   filled by the Claude provider from `RawLogEntry.cwd`. The daemon already stores the normalized
   record as `raw`, so the hub reads `m.raw->>'cwd'` with no migration. Codex and Pi keep the folder
   at turn level; filling it for them is a follow-up, and the spec claims the signal for Claude only.
   No re-ingest: existing rows stay without `cwd`, and the spec says the signal covers new data.
2. **One flatten (M4).** `flatten(message)` returns `(line_capped, view_parts)`: the transcript line
   with today's caps, and the uncapped parts the view needs (full text, tool-input values, tool
   results, `cwd`). Every capped line is a substring of the message's view text, which a property
   test asserts over every block type the providers emit.
3. **The view covers the session, not the day (M5, M6).** The routing pass reads each session in
   the group unwindowed, sidechains included. The transcript pass keeps the day window and skips
   sidechains as today. A session protected on any day is therefore protected on every day.
4. **Where markers and terms match (M2, M8).** Path markers: project path, every `cwd`, every
   tool-input string value, and the full text (tool results, summaries, user and peer text). Terms:
   the full text. Before matching, strings are normalized: URL-decoded, `\\` folded to `/`,
   lower-cased; JSON tool inputs are walked value by value (no `json.dumps` text); a Glob/Grep
   `path` is joined with its `pattern`; a marker of the form `/<name>` also matches `<name>` as a
   path segment with no leading or trailing slash. The encoded storage form and the org URL form
   are covered by routing-file entries, not by code. Name-only shapes (an alias, `z <repo>`) need
   repo identity (#11) or a term; the spec does not claim them.
5. **Send-time gate (M4).** On the general route, the last step before the call checks the exact
   prompt string for every marker and term; a hit protects the group (class `prompt`).
6. **Fail closed (M9).** Protect the group, with a class, when: a session read raises or returns
   fewer messages than the hub reports (`unreadable`); a message has a content shape the flatten
   does not know (`unreadable`, and the next group goes on); the hub's messages carry no `cwd` key
   at all once the hub is at or past this release (`cwd_unavailable`; `null` stays "no signal");
   the routing file is missing, invalid, or below its recorded floor or fingerprint (`protect_all`,
   #13). Matching is per message and stops at the first hit.
7. **Log classes only.** `path`, `cwd`, `cwd_unavailable`, `tool_input`, `text_marker`,
   `text_term`, `prompt`, `unreadable`, `protect_all`; never the marker, term or matched text.
8. **The open-threads feed (M7).** Newly protected groups write threads into lab projects. #12's
   origin filter (threads from protected entries offered only to protected calls) is a release gate
   for this change: it ships with it or before.

## Alternatives considered

- Untruncated prompt: rejected, the prompt budget exists for the model call.
- Hub-side routing flag: rejected for now; the markers live outside this repo and off the hub.
- A per-session protected flag stored at the hub (M6): heavier than reading the session
  unwindowed, which needs no new state; revisit if long sessions make the read slow.
- Separate `text_markers` in the routing file: deferred; split only if short markers prove noisy
  in the per-class counts. A false positive costs a protected call, not confidentiality.

## Risks

- More protected calls (the point), all through the protected `claude -p`; #14's hardening is
  recommended to land with this change.
- Unwindowed reads cost more hub reads for long sessions; measured on the first night.
- Older data has no `cwd`; the other signals cover it.
