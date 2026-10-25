## 1. Engine

- [x] 1.1 `prefix` skips `AKIA…EXAMPLE`; test with a random key id beside it.
- [x] 1.2 `Finding.origin`, set by `redact_json` with the same block rules as the dry run's `Origin` (shared `block_kind` / `child_kind`); `FieldHits.origins`.
- [x] 1.3 Migration 0015 `credential_findings.origins`; ingest and `findings redact` write it.
- [x] 1.4 `origins_by_rule` in the summary and health body.
- [x] 1.5 Tests: the example key, the bare PEM header, origins per block kind, an ingested tool result end to end.

## 2. Release and ops

- [ ] 2.1 ac/security approves the example-key exemption (scan policy).
- [ ] 2.2 Merge after `findings-ack`; release and swap with it or after it (a1024-02).
