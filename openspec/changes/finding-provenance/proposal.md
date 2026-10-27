## Why

On 2026-10-24 Gatus `cchv-findings` went red on one message (Gitea #82). Triage had to
open the client-data owner's transcript to learn two things the finding could have said
itself: the value was AWS's published documentation example key, and it sat in a
tool result (a fetched docs page), not in text a person typed. Every user-role row
looks the same today, because a Claude tool result is also stored with role `user`.

## What Changes

- The `prefix` rule no longer fires on AWS's documentation example access keys: a whole
  `AKIA` match ending in `EXAMPLE`. AWS issues random key ids, so a real one ending in
  that word is a 1-in-32^7 accident. No other example is exempted.
- Every finding from a JSON field records the content-block kind of each hit (`text`,
  `tool_result`, `tool_input`, `tool_use`, `other`). Migration 0015 adds
  `credential_findings.origins`; ingest and `findings redact` write it. `search_text` has
  no blocks, so its rows stay empty.
- The findings summary and health body gain `origins_by_rule`: per rule, distinct
  messages by `<role>:<origin>`, e.g. `user:text` for what a person typed and
  `user:tool_result` for a fetched page or a file a tool read.
- The `prefix` rule learns the client-key prefixes infra issues (infra-cpa, 2026-10-26): `sk-bf-`, `sk-mw-`, `aiproxy-ws-` and `aiproxy-pi-`, each followed by 30 or more of `[A-Za-z0-9_:-]`. A colon in the tail no longer ends the match, so no part of a key stays in clear. A file name that starts with one of these prefixes, or with GitLab's, is not a key (lowercase hyphenated words). On 2026-10-26 an `aiproxy-pi-` key was stored in clear because no rule knew the prefix.
- A test pins that a bare PEM header (no body, no END line) fires no rule. It already
  fired none; the 2026-10-24 hits were all `prefix`.

## Impact

- Hub binary and one migration, stacked on `findings-ack` (0014). Same release path; the
  rollback adds `DELETE FROM _sqlx_migrations WHERE version = 15`.
- The example-key exemption and the new prefixes change what the detector redacts, so
  ac/security approves both before merge (scan policy is theirs).
- Rows written before 0015 report no origin; nothing is backfilled.

- A PEM block whose body holds fewer than 40 base64 characters (`...`, `<key>`, empty) is
  not a key (ac/security 2026-10-27): the smallest real private key body, Ed25519 PKCS#8,
  is 64.
- The file-name exemption no longer covers a tail shaped like a UUID or holding a hex run of
  8 or more with a letter in it (ac/security 2026-10-27): every minted Bifrost key is
  `sk-bf-` + a UUID, and about 1 in 2,600 has a first segment of letters a-f only. A run of
  digits alone (a date, an id) still reads as a file name, as GitLab backup names need.

## Approval

ac/security approved the example key, the short-body PEM exemption and the new prefixes on
2026-10-27, on condition of the UUID fix above (now in). Each change in what the detector
redacts goes through the usual live canary.
