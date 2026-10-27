## 1. Engine

- [x] 1.1 `prefix` skips `AKIA…EXAMPLE`; test with a random key id beside it.
- [x] 1.2 `Finding.origin`, set by `redact_json` with the same block rules as the dry run's `Origin` (shared `block_kind` / `child_kind`); `FieldHits.origins`.
- [x] 1.3 Migration 0015 `credential_findings.origins`; ingest and `findings redact` write it.
- [x] 1.4 `origins_by_rule` in the summary and health body.
- [x] 1.6 Infra client-key prefixes (`sk-bf-`, `sk-mw-`, `aiproxy-ws-`, `aiproxy-pi-`), colon-inclusive; file-name exemption generalised from GitLab's.
- [x] 1.7 Security's conditions (2026-10-27): PEM body under 40 base64 chars exempt; file-name exemption refuses a UUID tail or a hex run of 8+ with a letter.
- [x] 1.5 Tests: the example key, the bare PEM header, origins per block kind, an ingested tool result end to end.

## 2. Release and ops

- [x] 2.1 ac/security approves the example-key exemption and the new prefixes (scan policy). Approved 2026-10-27 08:31Z with the UUID condition, which 1.7 meets.
- [ ] 2.3 Live canary after the swap: the AWS example and a short-body PEM kept; a UUID-tail `sk-bf-` key, a colon key and a 64-char PEM body redacted.
- [ ] 2.2 Merge after `findings-ack`; release and swap with it or after it (a1024-02).
