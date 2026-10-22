## 1. Engine

- [x] 1.1 `Finding` gains `reference` and `placeholder`; `assignment_finding` sets them and keeps a reference out of `assign-strong`.
- [x] 1.2 `FieldHits` counts reference and placeholder hits; migration 0013 adds `reference_hits` and `placeholder_hits` to `credential_findings`; ingest and `findings redact` write them.
- [x] 1.3 `flagged_messages_by_rule` in the summary and health body.
- [x] 1.4 The dry run takes the flags from the `Finding` (one source of truth).
- [x] 1.5 Tests: the two scenarios, the flagged count, and a re-sent batch adding nothing.

## 2. Release and ops

- [ ] 2.1 Merge the branch by PR; release cchv-v0.28.1; hub swap via infra-disk (binary only, `hub.toml` unchanged).
- [ ] 2.2 Live canary: strong fake redacted, a strong value with a reference inside left byte-identical (rule `assign`, reference 1), a placeholder-bearing strong value redacted; read `flagged_messages_by_rule`.
- [ ] 2.3 Bell manager-lab and security; the sweep (a1022-07) then waits on ac's yes.
