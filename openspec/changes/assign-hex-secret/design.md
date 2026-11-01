# Design

## D1. Derived from `assign`, after `assign-strong`

`assignment_finding` already decides `assign` and upgrades to `assign-strong`. A hit that is not
strong and has a hex value of 32+ under a word-final secret key becomes `assign-hex-secret`. A
mixed-case hex value can be 3 classes and so already `assign-strong`; strong wins (both redact).
Selecting `assign` also selects `assign-hex-secret` (a subset, like `assign-strong`).

Key ending, not containing: `sha256 =`, `digest =`, `commit =`, `checksum =` never match, and
`token_count`-style keys stay denied by `DENIED_KEYS`. A git sha in prose has no key at all.

## D2. Flag first

The rule ships flag-only. Security asked for its archive dry-run count before it redacts; infra
runs `cchv-hub findings dry-run --rule assign-hex-secret`, security reads the count, then infra adds
`"assign-hex-secret"` to `[redaction] redact`.

## D3. Volume count

`messages_by_rule_all` gets its own query: distinct messages per rule in the window, excluding
`api-request-blob` copies (as today) and excluding findings acked `false_positive`. Rows acked
`handled` stay in. `messages_by_rule` keeps excluding every acked row (it is what Gatus pages on
with `has()`), so acking still clears the cchv-findings check.

## D4. Placeholders

`GENERATE_…`/`EXAMPLE_…`/`YOUR_…`/`CHANGE_ME…` as one ALL_CAPS word (`^[A-Z0-9_]+$`) only.
Regex lines and format strings are not exempted (security): they are acked.
A value starting with the hub's marker is already a placeholder; re-quoted marker shapes are pinned by a test.
