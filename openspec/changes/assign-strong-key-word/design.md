## Context

`strong()` in `crates/hub/src/redact.rs` decides `assign-strong` from the value alone: from 32 to 63
characters it needs all four classes. The key is already on the finding (`Finding::key`), so the rule can use it
without new plumbing.

## Decisions

1. **The key word must be at the end** (option a′, security 2026-11-01, ac/security 42). The dry run showed
   "contains the word anywhere" (option a) is almost every secret key, since the assign pattern only matches keys
   that hold a secret word. Most of its extra volume was paths and URLs under keys like `*_token_url`. The
   word-final test keeps every value of the gap's shape (8 of 8) and halves the path-shaped ones.
2. **Words:** `PASSWORD`, `PASSWD`, `TOKEN`, `SECRET`, `KEY`, matched by `to_ascii_uppercase().ends_with`. That
   covers `NATS_PASSWORD`, `api_key`, `apiKey`, `client_secret` and `GITHUB_TOKEN`, but not
   `password_reset_url`. `KEY` alone also takes keys like `ssh_public_key`, a public value. That's accepted:
   it fails toward redaction.
3. **Exactly three classes.** Four already counts, and two (a hex digest) never does.
4. **References stay out**, as before: `hit.reference` is checked by the caller.

## Risks

- A bare snake_case identifier with a digit (32+ characters) under such a key is redacted. A call or a dotted
  path stays exempt (`code_expression`). Pinned by a test, so a later change sees it.
- Stored rows don't change until a backfill sweep, which needs manager-lab's go.
