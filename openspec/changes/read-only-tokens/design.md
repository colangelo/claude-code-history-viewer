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
