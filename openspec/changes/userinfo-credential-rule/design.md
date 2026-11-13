## Context

`crates/hub/src/redact.rs` holds a table of regexes (`PATTERNS`, parallel to `RULES`) and derived rules
(`assign-strong`, `assign-hex-secret`, `pair`). A finding's span is the secret value; overlapping spans share
the leftmost marker. `hub findings dry-run|redact` take `--rule`, and `messages_by_rule_all` lists `Rule::ALL`.

## Decisions

### D1 — A direct pattern, value-only span

One more regex, `scheme://user:(?P<value>secret)@host`, appended to `PATTERNS`/`RULES` (index 5; `EXTRA_ASSIGN`
stays 4). The span is the secret alone, so the marker leaves scheme, user and host readable
(`https://ac:[REDACTED:userinfo]@…`) and a repeat scan of redacted text finds nothing (the marker contains `[`,
which the value class excludes).

*Alternative rejected:* widening `assign` to URLs. It is key-name driven; a URL has no key, and the second-pass
comment in the file records why widening that pattern swallowed inner hits.

### D2 — Placeholder policy borrowed, not invented

A hit is dropped when the shared `placeholder()` or `mask_or_placeholder()` helpers say so, when the value elides
its middle (`..`, `…`, `*`), or when it equals one of eight example words. Templates never match at all, because
`{`, `}`, `<`, `>` are outside the value class. A real 2-day scan found 7 rows, 3 of them `ab12...cd34` display
masks; those are what the elision rule removes.

### D3 — Flag-only by default

Adding `userinfo` to the default `redact` list is a scan-policy decision (security). The rule exists and is
selectable; the one-off `hub findings redact --rule userinfo --expect ROW --max-rows N` handles the existing leak
once the row's owner agrees.

## Risks / Trade-offs

- **A public URL password in docs** (`https://user:s3cr3tpass@example`) is flagged; accepted, flag-only.
- **Value class excludes `:`, `[`** so a password with those characters is cut or missed; percent-encoded
  passwords (the URL norm) are covered. The detector is a floor, not a guarantee (stated in every dry run).
- **Performance:** the pattern begins with a scheme literal and `://`; the prefilter `RegexSet` rejects most
  text early. The existing scan benchmark covers the cost.

## Migration Plan

None. Ship the hub binary; flag-only findings begin at the next ingest. After security's call, add `userinfo`
to `[redaction] redact` in `hub.toml`.

## Required tests

| Contract | Verifier | Required result |
|---|---|---|
| `userinfo` is a rule id, not a subset of `assign` | `redact::tests::userinfo_is_a_rule_id` | `from_id`, `ALL` and `id` agree; selecting `assign` does not select it |
| A credential-store line is redacted, once, without leaking | `redact::tests::userinfo_redacts_a_bare_hex_secret_in_a_credential_store_line` | Exact marker output, no value in findings, idempotent |
| Flag-only when unselected | `redact::tests::userinfo_flags_without_redacting_when_the_rule_is_not_selected` | Text unchanged, one finding |
| Database URLs, JSON strings and a wrapped host | `redact::tests::userinfo_sees_a_database_url_inside_json_and_a_wrapped_host` | Value gone, marker kept before `@` |
| Negative controls | `redact::tests::userinfo_ignores_placeholders_short_secrets_and_userless_urls` | No `userinfo` finding for placeholders, masks, example words, userless and port-only URLs |
| Overlap with `prefix` | `redact::tests::userinfo_and_prefix_share_one_value_without_leaking_it` | Both findings, value redacted once |
| Ingest stores the marker in all three fields | `credential_findings_test::userinfo_rule_redacts_a_credential_store_token_in_all_three_fields` | `raw`, `content`, `search_text` redacted; findings row marked redacted |
| Volume summary lists the rule | `credential_findings_test` (`messages_by_rule_all` assertion) | `userinfo: 0` present |
| Real archive: positive control and false-positive census | `hub findings dry-run --rule userinfo --since 2d --expect <leak row>` | Verdict names the control found; non-leak hits are classed |
| Regression gate | Documented Rust gate (`cchv-deploy`) | fmt, clippy `-D warnings` (1.99.0), hub tests |
