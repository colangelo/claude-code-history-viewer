## Why

ac/security owns the credential-scan policy since 2026-10-22, and its G3 decision (ac/security
`docs/reviews/2026-10-22-cchv-assign-rule.md` § Decision) is binding. A 24 h study showed the
`assign` rule is mostly code and path shapes, so it must never page. But a strong-shaped value
behind a secret-named key is likely real and should be redacted at ingest. Today `assign` is
flag-only, so those values are stored in clear.

## What Changes

- **New derived rule `assign-strong`.** An `assign` hit whose value is 32+ characters with all
  4 character classes, or 64+ characters with 3+ classes, is recorded as `assign-strong`
  instead of `assign`. No new pattern: same key and value rules, different shape.
- **Redaction.** `redact = [..., "assign-strong"]` replaces only those values, with the fixed
  marker `[REDACTED:assign-strong]`, never a same-length mask. Selecting `assign` keeps
  covering `assign-strong`, for `[redaction]` and for `findings dry-run --rule`.
- **Health counts.** `/v1/healthz/findings` and `/v1/findings/summary` gain
  `messages_by_rule`: distinct messages per rule in the window, with `api-request-blob` copies
  excluded. That is the daily `assign-strong` count monitoring can alert on, counting each copy
  once.
- **Unchanged.** New messages only: no sweep of stored rows (that is ac's decision). Paging
  stays in Gatus, which never pages on `assign`.

## Impact

- `crates/hub/src/redact.rs`, `crates/hub/src/findings.rs`, and tests. No migration.
- Ops: `hub.toml` gets `"assign-strong"` in the same step as the binary swap. The old binary
  rejects unknown rule ids at start, so a rollback reverts `hub.toml` too.
