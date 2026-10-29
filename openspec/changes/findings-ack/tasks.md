## 1. Engine

- [x] 1.1 Migration 0014 `finding_acks` (`IF NOT EXISTS`, FK to `credential_findings` with `ON DELETE CASCADE`).
- [x] 1.2 `findings::ack` with `AckOptions` and `AckReport`; the note runs through the detector and is refused on any hit.
- [x] 1.3 CLI `hub findings ack`.
- [x] 1.4 Summary and health check exclude acknowledged findings and add `acked_by_rule`.
- [x] 1.5 `?days=N` adds `daily`.
- [x] 1.6 Tests for each scenario.

## 2. Release and ops

- [x] 2.1 Merge by PR; release; hub swap via infra-disk (binary only, `hub.toml` unchanged), with the rollback SQL in the relay.
- [x] 2.2 After the swap, acknowledge the 2026-10-22 sweep window and any finding the owner has triaged, on manager-lab's word; bell security with the `daily` shape for ac/security#14.

Shipped in cchv-v0.30.0 (tag 64714d42), live on m4m 2026-10-29 22:05:42Z. Acks the same night: handled 3,116 (the 10-22 sweep window 3,104; the 10-26 leak rows 12), false_positive 56 (docs page, pem template, and owner-triaged findings of 10-29).
