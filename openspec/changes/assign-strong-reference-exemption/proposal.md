## Why

ac/security set the policy for the `assign-strong` rule on 2026-10-22 (security review
`docs/reviews/2026-10-22-cchv-assign-rule.md` § Policy: exemptions):

1. A value holding an unexpanded reference (`${VAR}`, `$(cmd)`, `%VAR%`, `{{ var }}`,
   `#{var}`) is EXEMPT from `assign-strong`: it is not itself the secret, so it stays a
   plain `assign` hit, flagged, never redacted under `assign-strong`.
2. A value holding a placeholder word or shape is NOT exempt: a real secret can sit next
   to the word, so it is still redacted.
3. The daily count reports reference and placeholder hits separately, so the step-change
   alert can leave them out.

cchv-v0.28.0 shipped `assign-strong` without the exemption, so it over-redacts
strong-shaped references at ingest until this lands. The archive sweep (ask a1022-07)
runs under this policy, so old rows follow the rule that is live for new ones.

## What Changes

- `assignment_finding` no longer promotes a reference value to `assign-strong`.
- A `Finding` from the `assign` family carries `reference` and `placeholder` flags,
  computed from the value with `redact::is_reference` / `redact::is_placeholder`
  (added for the dry-run inventory on the same branch). Neither flag holds a value.
- `credential_findings` gains `reference_hits` and `placeholder_hits` (migration 0013),
  written at ingest and by `findings redact`.
- The findings summary and health body gain `flagged_messages_by_rule`:
  `{rule: {"reference": n, "placeholder": n}}`, distinct messages with at least one such
  hit, request copies counted once, as `messages_by_rule` counts them.

## Impact

- Hub binary and one migration; a release and a hub swap via infra. `hub.toml` is unchanged.
- `DETECTOR_VERSION` stays 1: rule ids are unchanged, and a row recorded by v0.28.0 keeps
  its rule (flag counts default to 0).
