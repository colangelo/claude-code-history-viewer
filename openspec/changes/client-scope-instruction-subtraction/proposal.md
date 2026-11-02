## Why

The client-scope classifier marks a session client when a path marker occurs anywhere in its stored
messages. Claude Code injects ac's global instructions file into every session: once as an
`instructions` attachment (the file's text in `raw`) and again inside the request copy
(`api-request-blob` rows, in `content`, `raw` and `search_text`). Two lines of that file name a
client folder, so every session that loads it is classed `text_marker` or `raw`, whatever the
session did.

Measured on pg1, 2026-11-02 (counts only): of the sessions active in the last 31 days, 357 are
`text_marker` and 158 `none`; 267 of the 357 carry an `instructions` attachment. The 11 sessions
that made the 14 Exa calls are among them, so the lab stats reader counts 0 Exa calls. The same shape
marked the first real GPT session (7a8c0cd7).

security (bells d457e4cf, 5064d033) and manager-aruba (dbb13776) accepted one design: do not skip
attachment or request-copy rows, but subtract only text that byte-matches a registered instruction
block, and scan everything else.

## What Changes

- The routing file's `[protected]` table can list `instruction_blocks`: each entry is the sha256 and
  byte length of one exact text (one version of an instructions file as Claude Code injects it), and
  the sha256 of its first 64 bytes.
- The classifier removes from a message string every span that starts at a line start and whose bytes
  hash to a registered block. It matches markers in what is left, piece by piece. Nothing is skipped by
  row type or block type.
- A block shorter than 256 bytes, or a malformed entry, makes the policy unknown (every session
  client), like any other invalid routing file.
- A new instruction block is a narrowing: it lowers what marks, so the hub treats it like a removed
  marker and holds the policy unknown until the audited `client-scope accept-routing`.
- `hub client-scope instruction-entry FILE` prints the entry for a file, as injected (trailing
  whitespace trimmed).
- `hub client-scope census --routing FILE --floor FILE` re-classifies, read-only, the sessions the hub
  has stamped client and prints how many would land in each class, and the ids of those that would be
  lab. It writes nothing.

Not changed: a session's class still only rises. A session marked before the registry exists stays
client until the audited `client-scope clear`; the census is the input for that decision.

## Capabilities

### Modified Capabilities

- `archive-search-api`: the client classification subtracts registered instruction blocks.

## Impact

- `crates/hub/src/client_scope.rs` (subtraction), `client_policy.rs` (parsing, acceptance),
  `client_state.rs` (narrowing message), `client_sweep.rs` (census), `lib.rs`, `main.rs` (CLI).
- The routing file and its floor record are manager-aruba's: registering a block is their edit, and
  the floor record's sha256 pins it.
- The distiller (`scripts/cchv-distill.py`) ignores the new key and keeps routing such sessions to the
  protected model, the safe direction. It can adopt the same subtraction later.
