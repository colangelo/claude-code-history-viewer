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

- [x] 4.1 Release with the Q-a modes. Infra deploys per `docs/archive/deployment.md`:
      `[redaction] redact = ["pem", "prefix"]` in m4m's `hub.toml`, then a hub restart.
      The detector itself shipped flag-only in cchv-v0.23.0.
      **Done 2026-10-10 with cchv-v0.24.0** (infra relay `7524e78b`): in the hub.toml
      template, re-rendered by `cchv-launch`; hub log 15:53:41Z `redact=["pem", "prefix"]`,
      no startup error.
- [ ] 4.2 Measure ingest latency before and after on prod (`/v1/ingest` p50/p95 from the
      hub log), positioned in the VACUUM cycle per the repo rule.
- [ ] 4.3 Infra/ac run `redact-existing --dry-run`, tune the `assign` deny list from the
      key-name counts, then flip the remaining rules to `redact`.
      **Now blocking 4.4:** until this lands, the findings check stays red at the measured
      rate (see 4.4). The key-name counts are already in `credential_findings.key_names`. A
      `GROUP BY` over the table (query in `docs/archive/deployment.md` § Credential findings)
      gives the tuning input without the full-scan dry run.
      **Key-name input, 2026-10-10.** Infra ran both queries on pg1 at 16:08:08Z,
      read-only (`BEGIN READ ONLY … ROLLBACK`; relay `d5ba399b`, thread `0e825314`): 181
      rows and 63 messages, detected 11:24:23Z..15:58:29Z. That is 4 h 34 min of data, not
      a day.
      - **No key-name tuning makes the check green.** By message count, most of it is the
        core names: `password` 11, `token` 10, `credential` 6, `secrets` 6, `SECRET` 3. A
        deny list cannot drop those, so at least 11 messages stay in this window whatever
        the list holds. `bearer` (2 messages) has no key to tune. The bound fails only if
        those messages are false positives, and the table cannot show that: it keeps key
        names, not value shapes.
      - **Deny-list candidates.** Together they cover at most 17 of the 63 messages, and
        fewer if another key shares a message:
        - Counts: `cache_creation_tokens` and `cache_read_tokens` (the list has only the
          `_input_` forms), `analytics.tokenUsage`, `pending.tokenLifetimeMs`,
          `credential_hits`.
        - Names of things, not secrets: `TokenEndpoint` and `token_endpoint` (URLs),
          `LOCAL_SECRET_FILENAME` and `local-secret.ts` (files), `AUTH_TOKEN_KEY` (a
          storage key), `TOKEN_SRC`, `secretRef`, and one tailnet hostname whose first
          label is `secrets`.

        Five of these are this repo's own identifiers (`git grep`), so some of the noise
        is sessions working on cchv. Keep `secret_id`, because an AppRole secret id is a
        credential. `localCredential` needs a look.
      - **As built, the dry run cannot get value shapes for this set.** `--since` filters
        on message `timestamp`, not `detected_at`, and a NULL never passes it. The busiest
        session in the window (14 of 63 messages) is a Cursor session. No archived Cursor
        session has message timestamps: 25 of 25, plus 3 Codex and 1 Antigravity session,
        so 29 sessions and 230 rows by the stored counter. That is our reading:
        `/v1/sessions` from ac-mbm5 at 16:18Z, all 5,351 sessions. Next: a dry-run scope
        over exactly the `credential_findings.message_ref`s in a window. The 63 messages
        then get value shapes without an id walk over the whole table.
      - **Where the hits are** (same reading): 38 of the 63 messages are in three m4m
        sessions. They are that Cursor session in a work project (14), a 9-minute
        headless run in a skill's directory (13), and a long-running session in this repo
        (11). The other 25 are spread over 11 sessions in 9 project directories. The
        value shapes decide which hits are real. Who rotates is not settled: Q-b and 4.5
        give it to infra, and infra's reply says it remains with ac.
- [x] 4.4 Gatus check per Q-b. The summary needs read-auth, which Gatus does not carry,
      so the check polls the unauthenticated `GET /v1/healthz/findings?since=24h`: same
      counts, `200` with none in the window, `503` with any. Code + test done 2026-10-10
      (branch `feat/34-findings-health`); the Gatus check itself is infra's, relayed with
      the release.
      **Deployed 2026-10-10** (infra `3219fc0`: `cchv-findings`, `[STATUS]==200`, 300 s,
      ntfy). **Red from the first poll, and it will stay red at the measured rate:** 181 rows in 24 h (`assign` 177,
      `bearer` 4), 21 in the last hour, one at 15:58:29Z, after the restart (read from
      ac-mbm5 at 15:59Z). We relayed the check in the same release, so it went live before
      4.3 had tuned the rule. While the check is red, a new finding does not change its
      state, so it pages no one. ac decides what pages until then (Q-b was his).
- [ ] 4.5 Infra runs the dry run on prod as the `ac/infra#104` sweep, with `--expect` set
      to the `#86`/`#102` locations, and owns every rotation. Its result is reported as
      a floor. Run it without `--since`. With it, every row that has a NULL message
      timestamp is skipped, and that includes every archived Cursor session (see 4.3).
