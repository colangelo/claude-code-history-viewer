## Why

On 2026-11-01 a subagent printed the relay password into a tool result (ac/security 41). The detector found it
as plain `assign` but did not redact it. The value was 32 random letters and digits with no symbol: three
character classes. `assign-strong`, the redacting form, needs all four classes from 32 to 63 characters, and
three only from 64 characters up. So a secret of the most common generated shape (alphanumeric, 32 to 63 characters)
reached the archive in clear text whenever it sat under a plain secret key.

## What Changes

- From 32 to 63 characters, an `assign` value with exactly three character classes becomes `assign-strong` when
  the key name ENDS in `PASSWORD`, `PASSWD`, `TOKEN`, `SECRET` or `KEY` (case-insensitive: `NATS_PASSWORD`,
  `api_key`, `apiKey`, `GITHUB_TOKEN`). A key that only contains the word elsewhere (`password_reset_url`,
  `token_endpoint`) keeps the current rule. Option (a′) of ac/security 42, chosen by security 2026-11-01.
- Unchanged: four classes from 32 characters, three from 64, references never strong, and the existing guards
  (placeholders, password hashes, bare code expressions, denied keys).
- No detector version bump. Earlier rule changes kept version 1, and the finding rows carry the rule.

## Impact

- `crates/hub/src/redact.rs` `strong()`, plus unit tests.
- Measured beforehand (dry run, SQL approximation over stored `assign` hits, search_text only): 149 values in 81
  messages would newly be `assign-strong`. That's 8 of the gap's shape, 47 base64-like, 61 containing `/`, and
  the rest other shapes. Security accepts redacting base64-like values under these keys as the right error.
- Stored rows are untouched by the release. A backfill sweep (`hub findings redact --rule assign-strong`) is a
  prod archive write: it goes to manager-lab for a go, with the hub CLI's own dry-run count first.
- Known accepted cost: a bare snake_case identifier that has a digit (three classes, 32 characters or more) under such a key is
  redacted. Calls and dotted paths stay exempt as code.
