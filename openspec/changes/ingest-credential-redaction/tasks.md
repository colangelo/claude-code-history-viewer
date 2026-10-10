# Tasks

> **Gate:** answered by ac 2026-10-10 (asks row a1009-03, "Q-a recs, Q-b Gatus"):
> `pem` and `prefix` redact from day one, `bearer` and `assign` stay flag-only, and a Gatus
> check pages infra to rotate on any new finding. Task 3.4 is left out: it reads journal text.

## 1. Fixtures and rule harness (no ingest change)

- [x] 1.1 Known-positive fixtures, one per rule and form: PEM (OpenSSH, RSA), each token
      prefix, `Bearer`, and `assign` in env, YAML, JSON (`bao kv get -format=json` shape)
      and NATS `authorization { … }` form. All values are **synthetic**: generated
      random strings, never a real credential, and never copied from the archive.
- [x] 1.2 Known-negative fixtures drawn from shapes this archive is full of: token counts,
      `max_tokens`, `tokenizer`, `input_tokens`/`cache_*_tokens`, placeholders (`$VAR`,
      `${VAR}`, `<token>`, `***`), already-redacted markers, and prose about passwords.
- [x] 1.3 Rule module with a `RegexSet` pre-filter. Tests assert every positive fires
      its rule and every negative fires nothing. Failure messages name the rule and the
      fixture id, never the value. Benchmark the per-message cost on a real day's batch
      shape (sizes only, synthetic content).
      **Done 2026-10-09** (`crates/hub/src/redact.rs`): 4 rules, 13 tests + 1 ignored bench;
      8 MiB synthetic scan 73 ms (debug build). Review added the code-expression guard
      (`let token = generate_token();`, `process.env.API_KEY`, `${{ secrets.X }}`).

## 2. Ingest integration

- [x] 2.1 Migration: `credential_findings(id, message_ref, field, rule, detector_version,
      detected_at)` with a unique key on `(message_ref, field, rule, detector_version)`
      so a re-sent batch adds nothing.
- [x] 2.2 Call the detector from `sanitize_batch`. In `redact` mode, replace the value in
      `raw`, `content` and `search_text` before any derivation. Record findings after the
      message rows exist.
- [x] 2.3 `hub.toml` `[redaction]` table: per-rule mode, default per Q-a. The detector
      version is a constant bumped whenever a rule changes.
- [x] 2.4 Integration tests (PG): flag mode stores unchanged plus a finding; redact mode
      stores the marker in all three fields plus a finding; a re-sent batch is unchanged
      with no duplicate finding; a token-count message yields nothing.

      **Done 2026-10-10** (branch `feat/34-ingest-credential-findings`, flag-only by default, so
      Q-a's per-rule modes are config, not code): migration `0011_credential_findings.sql`
      (also `hits`, `key_names`, `redacted`); ingest calls `redact::apply_to_message` per
      message before extraction and inserts findings only for newly inserted rows;
      `[redaction] redact = [...]` in `hub.toml` (unknown id = startup error; env-configured
      hubs are flag-only). Tests: `crates/hub/tests/credential_findings_test.rs` (6, PG).

## 3. Summary endpoint and retroactive tool

- [x] 3.1 `GET /v1/findings/summary?since=` → counts by rule, no text; read-auth like
      the other `/v1` reads.
- [x] 3.2 `cchv-hub redact-existing --dry-run [--rule] [--since] [--expect <sid>:<mid>]…`:
      id-batched scan. Per hit: rule, key name, session id, message id, host, timestamp,
      value shape. Totals by rule and key name. A reach section: scanned machines, time
      range, fields, rules, detector version, and the not-reached list (Mac-side JSONL, ingest
      gaps, outside `--since`, uncovered shapes). Verdict "could not look" when any
      `--expect` location is not hit. Tests: output contains no fixture value; a missed
      `--expect` flips the verdict; every hit carries its location.
      **Done 2026-10-10** as `hub findings dry-run [--since 7d|RFC3339] [--rule ID]...
      [--expect ROW|SESSION:UUID]... [--batch N]` (the `redact-existing` name is kept for the
      real run, 3.3). Read-only pool (`default_transaction_read_only`, 60 s statement timeout),
      no migration. Verdicts: UNVERIFIED (no `--expect`) / COULD NOT LOOK / FLOOR.
- [ ] 3.3 Real run: rewrite, insert findings, re-derive (tool rows, `message_embeddings`
      delete, journal day dirty, mirror rebuild note). Tested on the throwaway PG.
- [ ] 3.4 Flag-only scan of `journal_entries` text (design Q5 residual). **Out of scope
      (ac, 2026-10-10):** it would read journal text.

## 4. Rollout (each step needs ac/infra)

- [ ] 4.1 Release with the Q-a modes. Infra deploys per `docs/archive/deployment.md`:
      `[redaction] redact = ["pem", "prefix"]` in m4m's `hub.toml`, then a hub restart.
      The detector itself shipped flag-only in cchv-v0.23.0.
- [ ] 4.2 Measure ingest latency before and after on prod (`/v1/ingest` p50/p95 from the
      hub log), positioned in the VACUUM cycle per the repo rule.
- [ ] 4.3 Infra/ac run `redact-existing --dry-run`, tune the `assign` deny list from the
      key-name counts, then flip the remaining rules to `redact`.
- [ ] 4.4 Gatus check per Q-b. The summary needs read-auth, which Gatus does not carry,
      so the check polls the unauthenticated `GET /v1/healthz/findings?since=24h`: same
      counts, `200` with none in the window, `503` with any. Code + test done 2026-10-10
      (branch `feat/34-findings-health`); the Gatus check itself is infra's, relayed with
      the release.
- [ ] 4.5 Infra runs the dry run on prod as the `ac/infra#104` sweep, with `--expect` set
      to the `#86`/`#102` locations, and owns every rotation. Its result is reported as
      a floor.
