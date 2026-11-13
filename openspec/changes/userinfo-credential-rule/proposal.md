## Why

2026-11-13: a git credential store line (`https://ac:<40 hex>@gitea…`) was printed into a Claude transcript and
reached the archive in two rows. No detector rule fired: the secret is a bare 40-hex string with no
secret-named key, no known prefix and no `Bearer` word, so `assign`, `prefix` and `bearer` cannot see it, and
`pair` needs an object. The token sat in `raw`, `content` and `search_text`, searchable, and the hourly
distiller would have sent that session's text to a model. Credentials in URL userinfo
(`scheme://user:secret@host`) are a standard way for git remotes, database URLs and proxy settings to carry a
secret; the detector has no rule for the position.

## What Changes

- A new rule `userinfo`: the password part of `scheme://user:secret@host`, when it is 8 or more characters and
  not a placeholder, a template, a mask or a documented example word. It is a pattern of its own, so it is
  selectable in `[redaction] redact`, in `hub findings dry-run --rule userinfo` and in `hub findings redact`.
- `messages_by_rule_all` on `/v1/healthz/findings` gains a `userinfo` entry (an additive key).
- **Default policy unchanged.** `userinfo` is flag-only until security adds it to `[redaction] redact`; scan
  policy is security's (the engine is cchv's). The real run `hub findings redact --rule userinfo` works
  without that setting.

## Impact

Hub detector (`crates/hub/src/redact.rs`), the findings summary and docs. No migration, no schema change,
no new endpoint. New ingest findings appear as rule `userinfo`; paging thresholds that read
`messages_by_rule_all` keep working because the key is additive.
