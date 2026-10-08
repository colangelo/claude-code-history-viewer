# Proposal

## Why

The archive stores agent transcripts verbatim, so a credential an agent prints is durable at
rest on pg1. Two such leaks are known (`ac/infra#86`, `#102`). Both were found only because
the agent that caused them confessed, so the true count is unknown by construction (Gitea
#34). The control we own, settled with infra on 2026-08-17, is a **detector at ingest**.
Infra owns any sweep and every rotation (`ac/infra#104`).

## What Changes

- The hub scans each message as it arrives, before anything is derived from it, for
  credential **shapes**:
  - PEM private keys;
  - known-prefix tokens (GitHub, Anthropic, OpenAI, Slack, AWS, OpenBao/Vault);
  - `Bearer` headers;
  - secret-named assignments in env, YAML, JSON and NATS-config form.
- Each hit is recorded as a **finding**: message, field, rule, detector version. The finding
  stores **no value and no hash of one**.
- In **redact** mode, the matched value is replaced in `raw`, `content` and `search_text`
  with a fixed-length marker `[REDACTED:<rule>]`, before `search_text`, tool extraction,
  embeddings, the journal or the stats mirror ever see it. The key name stays, so a reviewer
  can tell what leaked without seeing it.
- Rollout is **per rule**: every rule starts in `flag` mode (detect and record only), and is
  switched to `redact` after its false-positive count has been measured on the live archive.
- New findings are surfaced: `GET /v1/findings/summary` returns counts only, so a Gatus check
  can tell infra to rotate. Redaction at rest does not un-leak a secret: it also passed
  through the model provider and is still in the source JSONL on the Mac.
- An operator command, `cchv-hub redact-existing`, applies the same detector to rows already
  ingested. `--dry-run` reports counts only. A real run re-derives everything built from a
  changed row. Whether and when to run it on prod is infra/ac's call, never this change's.
- **BREAKING (spec):** `raw` is no longer guaranteed to round-trip verbatim: redacted spans
  differ, and NUL bytes are already stripped today.

## Capabilities

### New Capabilities

- `credential-redaction`: credential-shape detection at ingest, findings without values,
  per-rule flag/redact modes, the findings summary, and retroactive redaction of stored rows.

### Modified Capabilities

- `archive-ingestion`: the *Normalized, raw-fidelity, and full-text storage* requirement's
  "stored verbatim" guarantee gains the sanitisation exception (NUL stripping, which already
  happens, and redaction).

## Impact

- **Hub:** `crates/hub/src/ingest.rs` `sanitize_batch` (the existing pre-insert pass) calls
  the detector. A new module holds the rules. A migration adds `credential_findings`. A new
  read endpoint and a new operator subcommand.
- **Derived data for retroactive redaction:** `messages.search_text`/`text_search`,
  `message_tool_uses`, `message_tool_results`, `message_embeddings`, `journal_entries` (day
  re-distill), `journal_embeddings`, and the DuckDB stats mirror.
- **Ingest latency:** regexes over every message. The budget is measured in tasks.
- **Daemon:** unchanged. `message_key` is computed daemon-side, so redaction doesn't
  disturb dedup, and re-sent batches redact identically.
- **Not covered, by design:** the source JSONL files on each Mac, and copies outside the
  archive. Those are infra's rotation scope.
