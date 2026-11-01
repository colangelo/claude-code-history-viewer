# Read-only hub tokens

## Why

ac said yes (asks a1101-25, 2026-11-01) to a token that lets one agent, the agents blog, read the
archive and nothing else. The hub cannot mint one: `crates/hub/src/auth.rs` knows two token kinds
(infra measured, 2026-11-01):

- full scope: reads everything, writes journal entries and distiller ticks, ingests under its
  machine id, and creates or deletes project identity aliases;
- `ingest_only`: ingests its own sessions and is refused everywhere else.

A full-scope token for a new machine id would let the blog write, which is not what ac approved.

## What Changes

- A token entry in `hub.toml` MAY set `read_only = true`. Such a token is accepted by every read
  endpoint and refused with 401 by every write: ingest, journal entries, distiller ticks, and the
  identity alias create and delete (which today sit behind the read extractor).
- The refusal does not depend on each write handler choosing the right extractor: a router layer
  refuses a read-only token on any method other than GET, HEAD and OPTIONS, so a write route added
  later refuses it with no extra code. The write extractors (`IngestMachine`, `AuthedMachine`)
  refuse it as well.
- An entry that sets both `read_only` and `ingest_only` is a configuration error; the hub refuses
  to start rather than guess which one was meant.
- The launcher (`scripts/cchv-launch.sh`) renders one read-only entry per reader from
  `kv/agents/cchv-readers/<reader>` (fields `token`, `machine_id`), with the same shape checks and
  skip-on-malformed behaviour as the guest entries.

Full-scope and ingest-only tokens behave exactly as before. The Tailscale identity read path is
unchanged (security rates it separately).

## Impact

- Code: `crates/hub/src/{config.rs, state.rs, auth.rs, lib.rs}`, `scripts/cchv-launch.sh`.
- Tests: `crates/hub/tests/token_scope_test.rs`, config unit tests.
- Spec: `archive-search-api` gains a requirement.
- Release: hub binary + launcher (cchv-deploys-ship-when-ready, after security's review); infra
  then mints the blog token at `kv/agents/cchv-readers/agents-blog`.
