# Design

## Context

Ingest (`crates/hub/src/ingest.rs`) already rewrites every message before insert:
`sanitize_batch` strips NUL bytes from `raw`, `content` and `search_text` and clamps
`search_text`. Everything else is derived after that pass, both inside ingest (tool
uses/results, `message_id`) and downstream: embeddings (`message_embeddings`,
`journal_embeddings`), the distiller's `journal_entries`, and the DuckDB stats mirror. A
transformation placed in `sanitize_batch` therefore reaches every copy the archive makes.

`message_key`, the dedup key, is computed by the daemon from the original record, so a
hub-side rewrite changes no identity. A re-sent batch hits `ON CONFLICT DO NOTHING` exactly
as before.

## Goals / Non-Goals

**Goals**
- The next leaked credential is **detected by the archive**, not confessed by the agent.
- Once a rule is in redact mode, the value never reaches pg1, the mirror or any derived table.
- No code path renders, logs, hashes or returns a detected value, including tests' failure
  messages.
- Every rule is proven against known-positive **and** known-negative fixtures before it ships.

**Non-Goals**
- Sweeping the archive or rotating anything: infra's scope (`ac/infra#104`).
- Redacting the source JSONL on the Macs, or the sync-daemon wire. Daemon-side scanning is a
  later optimisation (issue Q2); the hub is the one place an old daemon cannot bypass.
- Catching every secret. This is a shape detector. A password with no secret-named key
  around it is out of reach, and the summary says so.

## Decisions (the issue's five questions)

### Q1 — on a hit: redact and record; never refuse

| Option | Consequence |
|---|---|
| Refuse the ingest | The batch is lost and the daemon retries forever. The archive silently stops covering that machine, which is the failure the issue warns about. **Rejected.** |
| Store and flag | The secret stays at rest, which is the exposure we're trying to end. Acceptable only as a **trial** mode. |
| **Redact and record** | Lossy only in the secret's span. The finding row keeps the evidence that a leak happened. |

Marker: `[REDACTED:<rule>]`, the same length for every value, so neither length nor a
prefix survives. The key and the surrounding text stay, so `NATS_PASSWORD=[REDACTED:assign]`
still tells a reviewer what to rotate.

### Q2 — where: hub, in `sanitize_batch`

One implementation that no daemon version can skip, applied before any derivation. Daemon-side
scanning can come later to keep values off the wire.

### Q3 — false positives: narrow rules, value guards, measured per-rule rollout

v1 rules, each with an id used in findings and markers:

| Rule | Shape | Guard against the obvious false positives |
|---|---|---|
| `pem` | `-----BEGIN … PRIVATE KEY-----` … `END` | none needed; the whole block is replaced |
| `prefix` | `gh[pousr]_…{36,}`, `github_pat_…`, `sk-ant-…`, `sk-(proj-)?…{20,}`, `xox[abposr]-…`, `AKIA[0-9A-Z]{16}`, `hv[sbr]\.…{20,}` | length floors per prefix |
| `bearer` | `Bearer <token>` | token ≥ 20 chars, base64url/JWT charset |
| `assign` | key matching `PASSWORD`, `PASSWD`, `SECRET`, `TOKEN`, `API_KEY`, `PRIVATE_KEY`, `CREDENTIAL` (case-insensitive) followed by `=`/`:` in env, YAML, JSON or NATS-block form | value ≥ 8 chars; not all digits (`max_tokens: 4096`); not a placeholder (`$VAR`, `${…}`, `<…>`, `***`, `[REDACTED…]`); key not on a deny list (`tokenizer`, `token_count`, `tokens`, `input_tokens`, `output_tokens`, `cache_*_tokens`, `secret_name`) |

The `assign` deny list is the part most likely to be wrong, which is why rollout is per rule:
1. Ship with every rule in `flag` mode.
2. Run `redact-existing --dry-run` on prod (infra runs it). Per hit it reports the rule, key
   name, value shape (length bucket, character classes) and location, never a value, plus
   totals by rule and key name.
   A key name like `tokenizer` showing thousands of hits is an obvious false positive that
   can be read without seeing any secret.
3. Adjust the deny list, re-measure, then switch each rule to `redact` in `hub.toml`.

### Q4 — already-ingested data: a tool, not a decision

`cchv-hub redact-existing [--dry-run] [--rule …] [--since …]` scans `messages` in id batches,
using the same detector code as ingest. A real run rewrites `raw`/`content`/`search_text`
in place, inserts findings, and re-derives:
- `text_search` (generated);
- tool rows for that message (delete and re-extract);
- `message_embeddings` rows for that message (delete, so the embed sweep re-creates them);
- the journal day for that session (marked dirty, so the distiller re-runs, and its
  `journal_embeddings` row with it);
- finally a stats-mirror rebuild.

It never runs automatically. A dry-run result is a **floor, not a verdict**: it says what the
rules can reach, not that the archive is clean.

**The dry run is also the `ac/infra#104` sweep.** Infra agreed 2026-10-09 and runs it on prod
after the release; every rotation is theirs. They set three conditions, all binding:
1. **Locations, never values.** Each hit reports rule, key name, session id, message id,
   machine (host), message timestamp and value shape. Counts alone can't drive a rotation.
2. **Positive control first.** The run takes the locations of the two known real leaks
   (`ac/infra#86`, `#102`) as `--expect <session-id>:<message-id>`, repeatable. If it doesn't
   find every expected location, the report's verdict is **"could not look"**, not "clean"
   and not a count. This is the check that can fail: a dry run that misses a known leak
   proves the instrument is blind.
3. **Stated reach.** Every report says what it scanned (machines, time range, fields, rules
   and detector version) and what it could not: Mac-side JSONL never ingested, ingest gaps,
   rows outside `--since`, and shapes no rule covers.

### Q5 — embeddings and journal: covered by placement

New data is redacted before any reader exists. For old data, Q4's re-derivation list is the
answer. One residual stays: journal entries written *before* redaction may paraphrase a
secret in model prose. The detector runs over `journal_entries` text too, flag-only, and a
hit there marks the day for re-distill.

## Risks / Trade-offs

- **Ingest cost.** Several regexes per message over up to 8 MiB chunks. *Mitigation:* one
  `RegexSet` pre-filter pass, then the full rules only on fields that match. The
  per-batch cost is measured against a real day's batches before enabling (tasks 1.3, 4.2).
- **Redaction inside `raw` changes what "raw" means.** Accepted, and stated in the modified
  spec. NUL stripping already broke byte-verbatim.
- **A detector that silently matches nothing.** The known-positive fixture is the control:
  CI fails if any fixture shape stops matching (PASS/PASS/die discipline).
- **Leaking through our own tooling.** Findings hold no values; dry-run output holds no
  values; test assertions compare against redacted text, never print the input.

## Open Questions (ac / infra)

- **Q-a.** Is "flag first, redact per rule after measurement" acceptable, or should `pem` and
  `prefix` (near-zero false positives) ship in redact mode on day one? Rec: those two in
  redact mode on day one, `bearer` and `assign` flag-first.
- **Q-b.** Who gets notified on a new finding: a Gatus check on
  `/v1/findings/summary?since=24h` paging infra (rec), or a relay message per finding?
- **Q-c — answered (infra, 2026-10-09, recorded on `ac/infra#104`):** yes, the dry run is
  the #104 sweep, under the three conditions in Q4.
