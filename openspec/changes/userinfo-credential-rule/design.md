## Context

`crates/hub/src/redact.rs` holds a table of regexes (`PATTERNS`, parallel to `RULES`) and derived rules
(`assign-strong`, `assign-hex-secret`, `pair`). A finding's span is the secret value; overlapping spans share
the leftmost marker. `hub findings dry-run|redact` take `--rule`, and `messages_by_rule_all` lists `Rule::ALL`.

## Decisions

### D1 — A direct pattern, value-only span

One more pattern, `scheme://` (index 5; `EXTRA_ASSIGN` stays 4), then a hand parse of the authority
(`userinfo_secret`): last `@` ends the userinfo, first `:` opens the password, a user-only token is accepted by
shape. The span is the secret alone, so the marker leaves scheme, user and host readable
(`https://ac:[REDACTED:userinfo]@…`) and a repeat scan of redacted text finds nothing (an existing marker is
skipped). Exact repeats of a detected secret are replaced in the same text and, at message level, in every leaf of
`raw`, `content` and `search_text`.

*Alternative rejected:* widening `assign` to URLs. It is key-name driven; a URL has no key, and the second-pass
comment in the file records why widening that pattern swallowed inner hits.

### D2 — Narrow exemptions (security c1afb74, rows 1 and 2)

The first cut skipped placeholders by the shared helpers, example words and any `*` or `..`; security found that
suppressed real credentials with no finding. Now only an existing marker, a `$` variable, a whole-value run of `*`
and a short-ended display mask (`ab12...cd34`) are skipped; templates never match because `{`, `}`, `<`, `>` end
the authority. A real 2-day scan found 7 rows, 3 of them display masks.

### D3 — Flag-only by default

Adding `userinfo` to the default `redact` list is a scan-policy decision (security). The rule exists and is
selectable; the one-off `hub findings redact --rule userinfo --expect ROW --max-rows N` handles the existing leak
once the row's owner agrees.

## Risks / Trade-offs

- **A public URL password in docs** (`https://user:s3cr3tpass@example`) is flagged; accepted, flag-only.
- **Known misses, accepted:** no scheme (`//user:secret@host`), passwords under 8 characters, a secret split
  across a line wrap, escaped JSON nested inside a string, user names over 64 characters. The detector is a
  floor, not a guarantee (stated in every dry run).
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
| Negative controls | `redact::tests::userinfo_ignores_templates_masks_short_secrets_and_userless_urls` | No `userinfo` finding for templates, masks, short secrets, userless and port-only URLs |
| Real-looking secrets are not skipped (row 1) | `redact::tests::userinfo_flags_real_looking_secrets_that_resemble_placeholders` | `changeme`, `password`, `xxx…`, `R7k*…` flagged and redacted |
| Raw `@` in the password (row 2) | `redact::tests::userinfo_with_a_raw_at_sign_in_the_password_is_redacted_whole` | Whole password redacted; `host/path@name` untouched |
| Token in the user position (row 3) | `redact::tests::userinfo_sees_a_token_in_the_user_position` | 40/64-hex user flagged, with and without `:` |
| Query and fragment (row 5) | `redact::tests::userinfo_stops_at_query_and_fragment` | No finding; a credential URL with port, query, fragment loses only its password |
| Repeats within a text and across a message (row 4) | `redact::tests::userinfo_repeats_in_one_text_are_replaced_and_counted_once`, `redact::tests::userinfo_repeats_across_a_message_are_replaced_in_every_field` | No copy remains in any field; a near-miss token stays; one finding per URL |
| Linear scan (row 6) | `redact::tests::userinfo_scan_stays_fast_on_hostile_text` | Hostile `://` runs and many credentials scan within the bound |
| Overlap with `prefix` | `redact::tests::userinfo_and_prefix_share_one_value_without_leaking_it` | Both findings, value redacted once |
| Ingest stores the marker in all three fields | `credential_findings_test::userinfo_rule_redacts_a_credential_store_token_in_all_three_fields` | `raw`, `content`, `search_text` redacted; findings row marked redacted |
| Volume summary lists the rule | `credential_findings_test` (`messages_by_rule_all` assertion) | `userinfo: 0` present |
| Real archive: positive control and false-positive census | `hub findings dry-run --rule userinfo --since 2d --expect <leak row>` | Verdict names the control found; non-leak hits are classed |
| Regression gate | Documented Rust gate (`cchv-deploy`) | fmt, clippy `-D warnings` (1.99.0), hub tests |
