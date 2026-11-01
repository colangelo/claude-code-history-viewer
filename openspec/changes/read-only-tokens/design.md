# Design: read-only hub tokens

## D1. Writes refuse by method, not by extractor

The hub has three extractors: `IngestMachine` (ingest), `AuthedMachine` (journal entries,
distiller ticks) and `Authenticated` (reads). Two writes do not fit that split: the identity alias
create (`POST /v1/identities/aliases`) and delete (`DELETE /v1/identities/aliases/{id}`) take
`Authenticated`, because a trusted Tailscale identity (the webapp) makes them. If a read-only
token were only refused by the write extractors, those two would accept it, and so would any later
write route that takes `Authenticated`.

So the rule is enforced in two places:

1. A router layer: a request whose bearer token is read-only and whose method is not GET, HEAD or
   OPTIONS answers 401 before routing. This is the default deny: it covers registered and future
   routes alike and needs no per-handler code. OPTIONS stays open because a CORS preflight carries
   no credentials anyway.
2. The write extractors (`IngestMachine`, `AuthedMachine`) refuse a read-only token on their own,
   so the guarantee does not rest on one layer being mounted.

`Authenticated` accepts a read-only token. Its principal string stays `machine:<uuid>`.

Rejected: a separate `Reader` extractor opted in on each GET handler. It fails safe for new read
routes (401) but leaves the alias writes and any future `Authenticated` write open unless someone
remembers to change it; the method rule does not depend on memory.

## D2. One resolver, a scope argument

`resolve_machine(parts, state, allow_ingest_only)` becomes `resolve_machine(parts, state, scope)`
with `Scope::{Ingest, Write, Read}`:

| token kind | Ingest | Write | Read |
|---|---|---|---|
| full | yes | yes | yes |
| ingest_only | yes | no | no |
| read_only | no | no | yes |

## D3. Both flags fail the load

`read_only` and `ingest_only` together grant nothing. Rather than pick one, `HubConfig::load`
returns an error naming the entry (label, else machine id; never the token). The env path never
sets either flag.

## D4. Launcher

`kv/agents/cchv-readers/<reader>`, fields `token` (64 lowercase hex) and `machine_id` (lowercase
UUID), label = the reader name (`^[a-z0-9][a-z0-9-]{0,31}$`). Same loop shape as the guests:
a malformed or unreadable reader is skipped and logged, the others render. A machine id already
rendered (guest or reader) is skipped as a duplicate. The blog's sink reads only its own leaf.

## Non-goals

- The Tailscale identity read path (any process on m4m can read through `tailscale serve`):
  security rates it; unchanged here.
- Per-route read scopes (a token that reads only the journal): not asked for.

## Revision 2 (security review ac/security 8c52aa9, conditions C1-C3 before any mint)

cchv-v0.36.0 shipped revision 1 unminted (`read_only = true` on a `[[tokens]]` entry). Revision 2
replaces that format before any reader token exists, so nothing live changes shape.

### D5. Readers get their own table (C2)

`TokenEntry` ignores unknown fields, so under a rollback to a binary older than 0.36.0 a
`[[tokens]]` entry with `read_only = true` would load as a FULL token (Astra). Readers therefore
move to `[[readers]]` (`token`, `machine_id`, `label`, all required). An older binary does not know
the table, ignores it, and the reader token resolves to nothing: 401 everywhere. Both `TokenEntry`
and `ReaderEntry` get `deny_unknown_fields`, so `read_only` under `[[tokens]]` is a load error on
this and every later binary. The `read_only` field and the both-flags check go away; the structure
replaces them. `HubConfig` itself keeps accepting unknown top-level keys (the live template has
none, and refusing them would make every future table a rollback hazard in the other direction).

### D6. Duplicates are a hard error (C1)

Astra reproduced a reader leaf copied from a guest leaf: same token string, so the token kept its
guest scope. `HubConfig::validate` refuses any token string or machine id that appears twice across
`[[tokens]]` and `[[readers]]` (Mac template tokens included), naming entries by label or machine
id, never by token. The launcher checks the same over the whole rendered file and, on a duplicate,
exits without starting the hub and without reusing the cached runtime file: a cached file could
hold the very token being revoked or re-scoped. The guest and reader loops no longer skip a
duplicate machine id; they report it and the render fails hard.

### D7. Tests go through run()'s path (C3)

`AppState::from_config(pool, &HubConfig)` builds the token map, the trusted identities, the
ingest-only and reader sets and the redaction rules; `run()` and the scope tests both call it, and
the tests build `HubConfig` from TOML text. A `HubConfig::load` test (temp file through
`HUB_CONFIG`) proves `validate()` runs on load. Removing the reader set from `from_config`, or the
`validate()` call from `load`, fails a test.

### D8. P3s

A router test with `static_dir` set (the fallback answers 405 to an unknown POST; a reader still
gets 401). The launcher reads child names line by line (`while IFS= read -r`), never by word
splitting. `deployment.md` gets a revocation note: delete the leaf, relaunch the hub, and confirm
the rendered runtime no longer carries the label, because a launch that cannot reach bao reuses the
cached file.
