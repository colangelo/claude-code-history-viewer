# assign-hex-secret, and a volume count that leaves out false positives

## Why

1. **A detection gap (security P2, 2026-11-01).** Every cchv hub and guest token is 64 hex
   characters. Hex has two character classes, so `assign-strong` never fires on it, and `assign`
   only flags. Two old hub tokens sat in clear in 41 archived messages for four months, and the
   hub's search served one to an agent (frontend finding 405566622, manager-lab's leak row).
2. **A noisy alarm (security ruling, #65 comment 26620).** The findings volume count
   (`messages_by_rule_all`, read by infra's cchv-findings-volume check, 10 per 6 h) drops every
   acked row, including rows acked `handled` (a real secret = real exposure), and it pages on
   re-quotes of placeholders and of the hub's own redaction marker.

## What Changes

- New derived rule `assign-hex-secret`: an `assign` hit whose key ends in a secret word (`TOKEN`,
  `SECRET`, `PASSWORD`, `PASSWD`, `PASSW`, `API_KEY`, `API-KEY`, `APIKEY`, `BEARER`,
  case-insensitive) and whose value is 32 or more hex characters. Keys ending in `BEARER` or
  `API-KEY` are admitted for this rule even though they are not `assign` secret names. Flag-only
  until listed in `[redaction] redact` (infra adds it after security reads its dry-run count).
- `messages_by_rule_all` counts unacked rows and rows acked `handled`; only rows acked
  `false_positive` leave it. `messages_by_rule` (the paging presence map) is unchanged.
- Placeholders: a value that is exactly one ALL_CAPS word starting `GENERATE_`, `EXAMPLE_`,
  `YOUR_` or `CHANGE_ME` is a placeholder (never `assign-strong`).

## Impact

`crates/hub/src/redact.rs`, `crates/hub/src/findings.rs`, tests; spec `archive-ingestion`.
DETECTOR_VERSION stays 1 (a new rule id, as `assign-strong` was). Release: hub only.
