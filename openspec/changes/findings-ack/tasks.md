## 1. Engine

- [x] 1.1 Migration 0014 `finding_acks` (`IF NOT EXISTS`, FK to `credential_findings` with `ON DELETE CASCADE`).
- [x] 1.2 `findings::ack` with `AckOptions` and `AckReport`; the note runs through the detector and is refused on any hit.
- [x] 1.3 CLI `hub findings ack`.
- [x] 1.4 Summary and health check exclude acknowledged findings and add `acked_by_rule`.
- [x] 1.5 `?days=N` adds `daily`.
- [x] 1.6 Tests for each scenario.

## 2. Release and ops

- [ ] 2.1 Merge by PR; release; hub swap via infra-disk (binary only, `hub.toml` unchanged), with the rollback SQL in the relay.
- [ ] 2.2 After the swap, acknowledge the 2026-10-22 sweep window and any finding the owner has triaged, on manager-lab's word; bell security with the `daily` shape for ac/security#14.
