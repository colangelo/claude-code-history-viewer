# Tasks

> **Gate:** groups 2–4 wait on design.md Q-a/Q-b (ac); Q-c is answered. Group 1 is
> fixture work and safe to start.

## 1. Fixtures and rule harness (no ingest change)

- [ ] 1.1 Known-positive fixtures, one per rule and form: PEM (OpenSSH, RSA), each token
      prefix, `Bearer`, and `assign` in env, YAML, JSON (`bao kv get -format=json` shape)
      and NATS `authorization { … }` form. All values are **synthetic**: generated
      random strings, never a real credential, and never copied from the archive.
- [ ] 1.2 Known-negative fixtures drawn from shapes this archive is full of: token counts,
      `max_tokens`, `tokenizer`, `input_tokens`/`cache_*_tokens`, placeholders (`$VAR`,
      `${VAR}`, `<token>`, `***`), already-redacted markers, and prose about passwords.
- [ ] 1.3 Rule module with a `RegexSet` pre-filter. Tests assert every positive fires
      its rule and every negative fires nothing. Failure messages name the rule and the
      fixture id, never the value. Benchmark the per-message cost on a real day's batch
      shape (sizes only, synthetic content).

## 2. Ingest integration

- [ ] 2.1 Migration: `credential_findings(id, message_ref, field, rule, detector_version,
      detected_at)` with a unique key on `(message_ref, field, rule, detector_version)`
      so a re-sent batch adds nothing.
- [ ] 2.2 Call the detector from `sanitize_batch`. In `redact` mode, replace the value in
      `raw`, `content` and `search_text` before any derivation. Record findings after the
      message rows exist.
- [ ] 2.3 `hub.toml` `[redaction]` table: per-rule mode, default per Q-a. The detector
      version is a constant bumped whenever a rule changes.
- [ ] 2.4 Integration tests (PG): flag mode stores unchanged plus a finding; redact mode
      stores the marker in all three fields plus a finding; a re-sent batch is unchanged
      with no duplicate finding; a token-count message yields nothing.

## 3. Summary endpoint and retroactive tool

- [ ] 3.1 `GET /v1/findings/summary?since=` → counts by rule, no text; read-auth like
      the other `/v1` reads.
- [ ] 3.2 `cchv-hub redact-existing --dry-run [--rule] [--since] [--expect <sid>:<mid>]…`:
      id-batched scan. Per hit: rule, key name, session id, message id, host, timestamp,
      value shape. Totals by rule and key name. A reach section: scanned machines, time
      range, fields, rules, detector version, and the not-reached list (Mac-side JSONL, ingest
      gaps, outside `--since`, uncovered shapes). Verdict "could not look" when any
      `--expect` location is not hit. Tests: output contains no fixture value; a missed
      `--expect` flips the verdict; every hit carries its location.
- [ ] 3.3 Real run: rewrite, insert findings, re-derive (tool rows, `message_embeddings`
      delete, journal day dirty, mirror rebuild note). Tested on the throwaway PG.
- [ ] 3.4 Flag-only scan of `journal_entries` text (design Q5 residual).

## 4. Rollout (each step needs ac/infra)

- [ ] 4.1 Release with the Q-a modes. Infra deploys per `docs/archive/deployment.md`.
- [ ] 4.2 Measure ingest latency before and after on prod (`/v1/ingest` p50/p95 from the
      hub log), positioned in the VACUUM cycle per the repo rule.
- [ ] 4.3 Infra/ac run `redact-existing --dry-run`, tune the `assign` deny list from the
      key-name counts, then flip the remaining rules to `redact`.
- [ ] 4.4 Gatus check on `/v1/findings/summary?since=24h` per Q-b.
- [ ] 4.5 Infra runs the dry run on prod as the `ac/infra#104` sweep, with `--expect` set
      to the `#86`/`#102` locations, and owns every rotation. Its result is reported as
      a floor.
