# Design: hub client-read scope

Status: DRAFT for security's two-vendor review. No code until it is OK'd.

Author: cchv, 2026-11-02.

Origin: security R5 ruling, relayed by manager-lab (bell c78fc25a).

## Context

### What reads the hub today (security's list, bell e790f4ca)

| caller | credential | reads |
|---|---|---|
| distiller (m4m, launchd) | m4m machine token (bao `kv/infra/cchv/hub-tokens`, cached `~/.config/cchv/distill-hub-token`) | sessions whole, journal, open questions |
| cchv-find skill (any agent, any vendor) | machine token from bao, or 1Password | search, sessions, messages |
| webapp (ac's browser) | trusted Tailscale identity via `tailscale serve`, or an optional bearer token (ConnectGate, ArchiveHubSection) | everything |
| agents blog | `[[readers]]` token `agents-blog`, on `127.0.0.1:8790` | search, sessions, journal |
| any process on m4m or on ac's devices | the trusted Tailscale identity | everything |

Every row reads every session.

### How a session is known to be client today

Only the distiller knows. Its `route_group` (`scripts/cchv-distill.py`) matches the routing file's
path markers against these parts of every message of every session in the group: the project path,
each message's `cwd`, tool inputs and text. It also matches the content terms against the text.

- Every way of not knowing protects: an unreadable session, a partial read, an unknown content
  shape, a missing `cwd`, or a message or size bound.
- A missing, invalid or below-floor routing file protects everything. So does a file whose
  sha256 differs from the floor record.

Whatever the route, the hub stores and serves every session the same way.

## Goals

- **G1.** A caller without client read never receives client material: message text, session
  summaries, project paths, journal text, or stats tied to a client project.
- **G2.** Client read is held only by named principals that are Claude-only.
- **G3.** Fail closed. If the hub cannot decide whether a session is client, a lab-read caller
  does not get it.
- **G4.** No new route can leak by forgetting a filter: a test that walks the router catches it.
- **G5.** One rule for "client". The hub and the distiller match the same routing file with the
  same normalisation, and shared fixtures pin it.

## Non-goals

- The distiller's routing and the model it picks. Both are unchanged.
- Network policy for GPT sessions (dropping the hub's hostnames from their allow-lists): security's
  11-07 item. This design is the hub half; that is the network half.
- Copies made before enforcement, such as a GPT session that already read a client message.
- Telling a GPT process from a Claude process that runs as the same Unix user. The hub can only
  tell credentials apart. See R1.

## Decisions

### D1. The unit is the session, and the flag is sticky

A session's class is the first class its messages match, using the same classes as the distiller:

| class | matched against |
|---|---|
| `path` | the project path |
| `cwd` | a message's working folder |
| `tool_input` | a tool input value |
| `text_marker` | a path marker in message text |
| `unreadable` | an unknown content shape |
| `none` | nothing matched |

The class is stored on `sessions.client_class`, which is NULL while the session is unscanned.

- A session becomes client the first time any message matches. It stays client after that: later
  messages never clear it.
- A project is client when any of its sessions is. That rule is used only to hide project-level
  rows: the project list, stats per project, and identities.
- A journal entry is client when any of its `session_ids` is client, or when its `route` is
  `protected`.

**Why session and not message:** the messages around a client hit carry the same subject, so
hiding only the hit message would still leak. **Why session and not day group:** the hub serves
sessions, and the distiller already routes on whole sessions since v0.34.0.

**Content terms are NOT used by the hub** (alternative T below). Terms exist to keep a prompt away
from GPT, and there a false positive costs only a Claude call. On the night-1 run they matched
33 of 33 groups. As a read filter they would hide most lab history from every lab reader. Path
markers in text (`text_marker`) are kept: they name a client folder outright.

### D2. One matcher, pinned by shared fixtures

The hub gets a Rust port of the distiller's `normalize`, `marker_in` and segment rule, in a new
`crates/hub/src/client_scope.rs`.

- **Normalisation:** URL-decode, backslash to `/`, lower case.
- **Segment rule:** a `/<name>` marker also matches `<name>` as a path segment. The character before
  it must not be in `[a-z0-9_]`, and the character after it must not be in `[a-z0-9_-]`.

A fixture file, `scripts/fixtures/client-scope-cases.json`, lists inputs and expected classes with
made-up markers only. Both test suites run it:

- `cargo test` (hub);
- `scripts/test_cchv_distill.py` (distiller).

This is the same pattern as the journal protection tag (C4). If the two implementations drift, a
test fails.

The hub reads the routing file named by `hub.toml [client_scope] routing_file`. It runs the same
floor check as the distiller: `[client_scope] floor_file`, which holds the sha256 and the minimum
count of each marker kind. The routing file stays outside the repo, so no client name enters this
public fork.

### D3. Fail closed at three levels

1. **Hub start.**
   - *Enforce mode:* a missing, invalid or below-floor routing file, or one whose hash changed,
     puts the hub in `client_scope = unknown`. In that state every session counts as client, so
     lab readers get empty lists and 404s, and `/v1/healthz/client-scope` answers 503.
   - *Observe mode:* the same failure is logged and reported as 503 on that health route, and
     nothing is hidden.
2. **An unscanned session** (`client_class IS NULL`): treated as client.
3. **A routing file whose hash differs from the one recorded at the last full sweep**
   (`client_scope_state.routing_sha`): every session scanned under the old hash counts as
   unscanned until the sweep re-scans it. This is the case after a marker is added.

   The sweep runs in the background, oldest sessions first. Lab readers lose sessions while it
   runs and get them back as it scans, so access only narrows. Re-scanning is monotonic: a new
   marker can only add client sessions.

**Narrowing** is any change that would clear a flag: a marker removed, or a false positive cleared
by hand. It is a CLI command, `cchv-hub client-scope clear --session <id> --approved-by <who>`.
Like the distiller's `--narrowing-approved-by`, it takes an approval name. It is written to an
audit table and never runs automatically. The pg1 writer runs it on ac's row.

### D4. Computing the flag

- **At ingest.** `ingest.rs` matches each new message against the loaded markers before the batch
  commits, and sets `sessions.client_class` in the same transaction. Cost: one normalised substring
  scan per message over 21 short path markers. That is small next to the redaction pass ingest already
  does.
- **Backfill sweep.** `cchv-hub client-scope sweep [--dry-run]` re-scans unscanned sessions in
  batches. It reads `search_text` plus the `cwd` field of `raw`, and the project path. The dry run
  prints counts by class only. The first full sweep, about 20 M messages, runs once at the observe
  stage. infra runs it as the pg1 writer.
- **The distiller reads the flag.** `route_group` first asks the hub for the session's class. If
  the class is not `none`, it routes protected without reading the session. It keeps its own
  whole-session check for `none` sessions, because only the distiller matches terms. Both
  directions only add protection.

### D5. Scopes and principals

`Scope::Read` splits in two, and `Authenticated` carries which one the caller has:

```rust
pub enum ReadScope { Lab, Client }
pub struct Reader { pub principal: String, pub scope: ReadScope }
```

| principal | today | after |
|---|---|---|
| full machine token (`[[tokens]]`) | read all | lab read; `client_read = true` per entry gives client read |
| `ingest_only` token | no read | no read (unchanged) |
| `[[readers]]` token | read all | lab read; `client_read = true` per entry gives client read |
| trusted Tailscale identity | read all | lab read, never client read |
| no credential | 401 | 401 |

Client-read holders at launch:

1. **The distiller.** A new token `kv/agents/cchv-client-readers/distiller` replaces its use of the
   m4m machine token for reads. Its writes keep the machine token. It has to read client sessions,
   because it distils them on the protected route. It is a script, not a model. It sends client
   text only to the Claude id, and it refuses any other serving model.
2. **ac's browser.** A human client-read token, `kv/agents/cchv-client-readers/ac-browser`, pasted
   once into the webapp's existing token field (ArchiveHubSection, ConnectGate). Tailscale identity
   alone cannot give client read: the hub cannot tell ac's browser from any process of the same
   login on his devices. That is the R5 finding itself.
3. **The blog's client audience.** A second reader token, `kv/agents/cchv-client-readers/blog-client`,
   used only by the blog's Claude-only writer. The blog's existing `agents-blog` token stays lab
   read.
4. **Claude-only client sessions that need archive search** (manager-aruba's fleet, through
   cchv-find). One token per session family, each added on ac's row. None is minted at launch.

The launcher renders the `cchv-client-readers` children with `client_read = true`. It applies the
same shape checks as the other children, and any duplicate is fatal (as in v0.37.0). The bao policy
on that path is infra's: readable only by the distiller AppRole and by ac.

### D6. Enforcement in every read route, default deny

Every read SQL statement that touches sessions, messages, projects, journal entries or the stats
mirror takes a `Visibility` argument with no default:

```rust
pub enum Visibility { Lab, Client }
```

Each statement adds the predicate below when `Visibility::Lab`:

```sql
AND s.client_class = 'none' AND <scan current>
```

A helper `Visibility::session_predicate(alias)` builds it, so no handler writes it by hand.
Handlers get `Visibility` only from the `Reader` extractor.

| route | lab-read behaviour |
|---|---|
| `GET /v1/search` (messages and journal hits) | client sessions' messages and client entries left out; `X-Total-Count` counts only what is visible |
| `GET /v1/projects` | client projects left out |
| `GET /v1/sessions` | client sessions left out |
| `GET /v1/sessions/{id}/messages` | 404, the same as a missing id (it does not confirm existence) |
| `GET /v1/journal/entries` (browse and search) | client entries left out |
| `GET /v1/journal/open-questions` | threads from client entries left out |
| `GET /v1/journal/pending` | lab read lists pending days of non-client sessions only; client read (the distiller's new token) lists all |
| `GET /v1/stats/global` | totals unchanged (counts only); per-project breakdowns drop client projects |
| `GET /v1/stats/projects/{key}`, `/v1/stats/sessions/{id}` | 404 for client |
| `GET /v1/identities` | client projects left out |
| `GET /v1/findings/summary` | unchanged: counts by rule only, never a value, session or key |
| `GET /v1/healthz*` | unchanged: counts only |
| static webapp | unchanged: code only, and the data comes through the routes above |

`POST /v1/identities/aliases` and `DELETE /v1/identities/aliases/{id}` refuse a lab-read caller
that names a client project. They answer 404.

**Route-walk test (the oracle for G4).**
- *Setup:* seed one lab session and one client session, each with a unique canary string in its
  project path, `cwd`, text, summary and journal entry.
- *Run:* enumerate every GET route registered in `lib.rs` (the test reads the router, so a new
  route is included without edits) and call each with a lab-read token, the Tailscale identity,
  and a client-read token.
- *Assert:* the client canary appears in no lab-read or Tailscale response body, header or status
  that differs from the missing-id case. For client read, it appears wherever the lab canary
  appears.
- *Mutation check:* remove the predicate from any one statement, and the test must fail.

### D7. Rollout

| stage | what | gate |
|---|---|---|
| 0 | this design | security two-vendor review (Claude Opus plus GPT Astra on code and fake fixtures only) |
| 1 | build: migration, matcher plus fixtures, ingest flag, sweep CLI, `Reader`/`Visibility`, every route, route-walk test, launcher, distiller token | gate green; security review of the code |
| 2 | release in `observe` mode: flags computed, nothing hidden; infra runs `client-scope sweep --dry-run`, then the sweep | security reads the counts by class (no names, to manager-aruba too) |
| 3 | infra mints the client-read tokens (distiller, ac-browser, blog-client) on ac's row; the distiller switches tokens | read-back: the distiller still reads client sessions, the lab token gets 404 |
| 4 | `enforce = true` in `hub.toml` | probes: lab token, Tailscale identity and blog `agents-blog` get the client canary 404 or absent; client-read tokens see it |
| 5 | context points the blog's client audience at its own token | security closes R5 (hub half) |

Rollback: `enforce = false` restores today's reads with no data change, and the flags stay
computed.

## Risks

- **R1. A token held as a file is readable by any process of the same Unix user**, a GPT session
  included. The hub tells credentials apart, not models. The residual is closed only together with:
  - the GPT sandboxes' never-read list, which must cover `~/.config/cchv` and any client-read cache
    path (context's codex-delegate item);
  - security's 11-07 network item: no hub hostname, and no `127.0.0.1:8790`, in GPT sessions'
    allow-lists.

  Stated, not solved here.
- **R2. Matcher drift between Python and Rust.** Mitigated by D2's shared fixtures. A marker form
  one side does not know fails both test suites.
- **R3. Over-hiding.** `text_marker` may hide lab sessions that discuss a client folder (for
  example cchv's own sessions answering manager-aruba). This is acceptable, and it is fail-closed.
  The stage 2 counts show how much. Narrowing goes through D3's audited clear.
- **R4. Under-hiding.** A client session with no marker anywhere (the client never named in path,
  cwd or text) stays lab. The distiller has the same limit, apart from its terms. The stage 2
  counts against the distiller's protected entries show the gap.
- **R5. The sweep's cost on pg1.** About 20 M messages, run once, batched. infra picks the window.
- **R6. Ingest-time race.** A session is visible to lab readers until its first matching message
  arrives. If the marker comes late in a session (for example a `cd` into a client folder), lab
  readers could have read the earlier messages before the flip. Residual. The lab is open only
  until that message is ingested, about every 5 min.

## Alternatives considered

- **T. Content terms in the hub flag too.** Rejected for reads: they matched 33 of 33 day groups on
  night 1, so lab readers would see almost nothing. Security can still choose it. It is one switch
  in `client_scope.rs`.
- **Project flag from path markers only.** Simpler, but it misses client work done from a lab
  project's folder (`cwd`, `tool_input`, `text_marker`), which is the shape of the 09-22..09-26 case.
- **A separate client hub, or a separate database.** The strongest fence, but it needs two
  archives, a split ingest and two webapps. It is weeks of work, not days. It could be revisited if
  R1 cannot be closed.
- **Hide only messages that match.** Leaks the surrounding context (D1).
- **Tailscale identity for ac's client reads.** It cannot tell ac's browser from other processes
  of his login (the R5 finding itself).

## Open questions for security

1. Content terms in the hub flag: no (recommended), or yes (alternative T)?
2. `/v1/stats/global` totals including client sessions: OK as counts only, or subtract them?
3. Journal entries written by GPT on client days before routing existed (07-18..10-20): are they
   hidden from lab readers by the session rule (recommended), or redacted too?
4. Who approves a narrowing clear: ac's row (recommended), or security?

### Answers so far

manager-aruba, 2026-11-02 00:10Z (bell 312663ad), for the client side:

- **Q1 (terms):** security's call. manager-aruba's view: path markers only at launch, and R4
  accepted while the distiller keeps its own term check. Revisit with the observe-mode counts.
- **Q2 (totals):** counts-only totals are fine for lab readers.
- **Q3 (pre-routing entries):** hiding them by the session rule is enough, with no redaction. ac
  already decided to keep those rows (10-29), and a redaction would reopen that decision.
- **Q4 (narrowing):** for a client session, ac approves through the client desk's row. Never a lab
  session, a lab token or a lab desk row alone.
- **D5:** OK that `blog-client` (its Claude-only writer) is a client reader.
- **D7:** the observe-mode sweep counts go to manager-aruba, as written.
