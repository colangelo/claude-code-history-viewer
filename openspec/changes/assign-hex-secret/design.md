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

## Revision 2 (security review ac/security fa3c806, E1-E4)

- **E1, two passes.** Adding PASSW, API-KEY and BEARER to the shared `assign` pattern made new outer
  candidates (`passw_hint=TOKEN=<strong>`); the outer one is rejected and, matches not overlapping,
  the inner live `assign-strong` secret was never visited. The original pattern is restored
  unchanged; the new key forms are a second `assign` pattern whose spans are kept only where no
  other span overlaps them. So the second pass can only ADD an `assign-hex-secret`; it cannot
  change what the live rules find or redact. Pinned by a test that fails on 80904182.
- **What the new key forms yield.** Under a key that only the second pattern matches (`x-api-key`,
  `bearer`, `db_passw`), only a hex value of 32+ is a finding (`assign-hex-secret`); any other
  value, strong or not, yields nothing, because plain `assign` stays gated by `secret_key()`. The
  review's note that assign-strong "now also redacts strong values under the new keys" does not
  hold with this design: those keys add hex secrets only. `assign` stays out of `redact`.
- **E2.** The placeholder word is checked on the untrimmed value: a spaced value still fires.
- **E3.** The "all digits is a number" exclusion does not apply to a hex secret: 32+ digits under a
  hex-secret key fire.
- **Follow-ups (not in this change).** F1: a redact-listed bearer-hex subset for
  `Authorization: Bearer <32+ hex>`. F2: bare values end at `&` in query strings.
