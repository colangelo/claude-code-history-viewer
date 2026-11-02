# Design: hub client-read scope

Status: REVISION 2, for security's single-reader re-check. No code until it is OK'd.

- Revision 1 (0757504b) had security's two-vendor review: ac/security
  `docs/reviews/2026-11-02-cchv-hub-client-read-scope.md`. The verdict was that the shape holds, the
  design needs revising, and then the build can start.
- The section **Revision 2** below answers the review's items D-A to D-J and the P3 notes.
- Where Revision 2 and D1–D7 disagree, **Revision 2 wins**.

Author: cchv, 2026-11-02.

Origin: security R5 ruling, relayed by manager-lab (bell c78fc25a).

## Revision 2 (security review, 2026-11-02)

**Live finding L1** (`/v1/healthz/journal` gave `project_path` to callers without a credential) is
fixed separately, in v0.38.1. A group's `project_path` now goes only to a read principal. Once this
change lands, that detail also passes through `Visibility`: a lab reader gets no path of a client
group.

### R2-A. Classify everything a route can serve, from the full stored form (D-A, P1)

**One extractor, `client_scope::Extract`, used by both ingest and the sweep.** It yields every
string a read route can serve:

| source | what is extracted |
|---|---|
| a message's `content` | in full, walked recursively: text, thinking, tool_use inputs (every string leaf), tool_result content (every string leaf, nested arrays included), and image or document blocks' names and paths |
| a message's `raw` | in full: every string leaf, keys included, so that `cwd`, `gitBranch`, `toolUseResult` and attachment paths are covered |
| the session row | `summary`, `file_path`, `entrypoint`, the project path |
| the project row | `project_path`, `name`, and the identity fields served by `/v1/identities` (git remote, worktree main path) |

- **What is not read:** the sweep never reads `search_text`. That copy is clamped at 64 KiB
  (`ingest.rs SEARCH_TEXT_MAX_BYTES`), and the routes serve `content` and `raw` in full.
- **Limits:**
  - per message: 16 MiB of extracted text, and a nesting depth of 64;
  - per session at ingest: no limit, because each batch is classified as it arrives.

  Past a limit, or for a JSON shape it does not know, the extractor returns `unreadable`, which is
  client. A field the hub stores clamped or truncated is classified from the unclamped value it
  received. If only a clamped copy exists, it is `unreadable`.
- **Metadata-only updates:** a batch that only upserts a session or project row (a new summary, a
  new file path) is classified in the same transaction as the update.

**Tests.** A marker in each of these places hides the session, both at ingest and in the sweep:

- past 64 KiB of text;
- only in a nested tool_result;
- only in thinking;
- only in `raw` (`cwd`);
- only in the summary;
- only in `file_path`;
- only in a metadata-only update to a lab session;
- in an unknown block shape (counts as unreadable).

### R2-B. Protection propagates across a session family (D-B, P2)

A session's *family* is every session that shares any of these:

- the same `(provider, provider session_id)`, from any machine (copies);
- a parent or child link: a sidechain or subagent transcript whose `raw` names the parent's
  provider session id, and the parent itself;
- a distiller group (`entry_date`, `project_path`) that a journal entry marked `route = protected`.
  Every session in that entry's `session_ids` is in the family.

**Rules:**

- A client class on any member makes every member client, with class `family:<member class>`.
- Propagation runs in the same transaction as the flag, at ingest and in the sweep, so the family
  can only grow.
- A link the hub cannot verify (a parent id that names no stored session) quarantines the child as
  `family:unverified` until the parent arrives and is classified.

**Tests:**

- two machine copies with the marker in one only: both are hidden;
- a sidechain-only hit hides the parent;
- a parent-only hit hides the sidechain;
- a protected journal entry hides all of its sessions;
- an orphan child stays hidden until its parent is ingested lab.

### R2-C. Policy freshness and scan coverage (D-C, P2)

- **Policy generation.** The routing file and the floor file are read as one snapshot:
  - generation = sha256 of (routing bytes ‖ floor bytes);
  - they are re-validated every 60 s, and at the start of each ingest batch.
- **A generation change** (the file is deleted, corrupted, extended or replaced) moves the hub to
  the new generation *before* any read under it. Every session scanned under an older generation
  stops being lab at that moment.
- **Scan watermark.** Each session keeps `client_gen` and `client_scanned_upto`: the highest message
  id and the session row revision covered by a complete scan under `client_gen`. A session is lab
  only when all three hold:
  - `client_class = 'none'`;
  - `client_gen` is the current generation;
  - the watermark covers every stored message and the current row revision.

  A harmless new message classified at ingest moves the watermark only when the session was already
  complete under the current generation. Otherwise the session stays unscanned until the sweep
  reaches it.
- **Sweep writes:**
  - `UPDATE … WHERE client_gen IS DISTINCT FROM $gen AND row_rev = $seen_rev`;
  - each write is monotonic (class only rises, from `none` to client), so a crash, a resume, or two
    sweeps at once cannot lower a flag.
- **Tests:**
  - delete, corrupt, extend and replace the file under a running hub: lab visibility is withdrawn
    within one interval;
  - add a marker that matches an old message, then ingest a harmless message into that session
    before the sweep: the session stays hidden;
  - pause a sweep around a matching ingest: the session ends client.

### R2-D. Narrowing: the hub keeps its own accepted marker set (D-D, P2)

- **The record.** Table `client_scope_policy` holds the accepted marker set as one sha256 per
  marker, plus the generation.
- **What the hub does at a generation change:**
  - markers only added: accept automatically; access only narrows;
  - any marker removed or replaced, same count included: the hub goes `unknown` (every session
    client, the health route 503) until `cchv-hub client-scope accept-routing --approved-by <who>`
    records the new set.
- **Logging and approval:**
  - every generation change logs one line, `client_scope policy change: added N removed M`, which
    monitoring can alert on;
  - the approval name is attribution, not authentication (as security says).
- **Who approves a narrowing:**
  - for any session under the client tree, or carrying a client marker: ac, on the client desk's
    row (the client manager, Q4);
  - never a lab session, a lab token or a lab desk row alone;
  - the audit table records the session or marker hash, the policy generation, the reason and the
    approver, and security reads it at each review.
- **Test:** a same-count marker swap with a matching floor record goes `unknown`.

### R2-E. Stats from visible rows only (D-E, P2)

- **Lab aggregates are built from visible sessions only,** before grouping and ranking. That covers
  totals, `most_used_tools`, `most_used_skills`, `most_used_subagents`, model names, date buckets,
  per-project and per-session stats. This replaces my earlier answer to Q2 (counts-only totals)
  for everything except true totals.
- **The DuckDB stats mirror carries `client_class` and `client_gen` per session.**
  - A lab stats query filters on them.
  - If the mirror's generation is not the hub's current one, or its refresh is older than the last
    flag change, lab stats answer 503 with the reason. They never serve a stale view.
- **Tests:**
  - a skill name used only in a client session is absent from lab stats;
  - a date with only client activity is absent;
  - a mirror frozen across a client flip stops serving that session to lab readers.

### R2-F. Journal provenance (D-F, P2)

**Entries.** An entry is client when any of these holds:

- any of its `session_ids` is client;
- `session_ids` is empty or names a missing session;
- `route` is `protected`;
- `route` is missing (every entry before v0.34.0);
- `route_at != generated_at`.

So every entry from before v0.34.0 (no route stamp) is hidden from lab readers. That is
fail-closed, and it follows Q3 (hidden, no redaction). The same goes for 11-01, whose entries
were all protected (review H6).

**The cost:** lab readers, the blog included, lose the whole journal history until it is
re-proven.

**The way back is one audited step per entry:**

- **The command:** `cchv-hub client-scope restamp-history --dry-run | --approved-by <who>` runs the
  distiller's current whole-session router over each historical entry's sessions. That is the full
  router: path, cwd, tool input, text marker and **content terms**. It is the same procedure as the
  10-21..10-31 dry re-route of 2026-11-02.
- **Only an entry where nothing matches** gets `route = 'general-restamped'` plus a current
  `route_at`. Anything else stays hidden.
- **Before it writes:** the dry run's counts go to security and the client manager first.
- **Security's additions (re-check of Revision 2, 2026-11-02):**
  - **The entry's own text** is classified too: summary, threads and every served field. An entry
    is restamped only when both its own text and its sessions match nothing.
  - **The write** happens only after ac approves it on one row, with the client manager's word. It makes
    material lab that is hidden today (Q4).
- **Stage 2 also counts** sessions hidden *only* by family from a term-protected entry (R2-B). Most
  sessions since 11-01 are in that set: fail-closed, and expected.

Security ACCEPTED Revision 2 (7cd5a699) on 2026-11-02, with the additions above. The build may
start, and security reviews the code before observe mode.

**Threads (open questions).** A thread's protection is computed over its full source history, before
the display window and the filters apply. The thread is client if any entry that opened, continued
or resolved it is client.

**Tests:**

- a route-less entry;
- a stale stamp;
- a missing session reference;
- a thread whose earlier source entry turns client after the thread was shown.

### R2-G. No existence oracles (D-G, P2)

- **Visibility is applied before id resolution:**
  - a provider session id that is ambiguous across machines counts only visible candidates, and the
    400 lists only visible numeric ids;
  - a provider id whose only matches are client gets the same 404 as an unknown id.
- **One missing-resource contract:** 404 with body `{"error":"not found"}`, the same for a hidden
  row and a missing row, with or without a time window.
- **Writes are authorized on their target before any data-dependent validation:**
  - journal POST, ticks: machine token, unchanged;
  - alias create and delete: a lab-read caller naming a client project gets that 404 before any
    duplicate or cycle check.
- **Tests:** differential (R2-J). Each response is compared with the same request after the hidden
  rows are physically deleted.

### R2-H. Where client-read credentials rest (D-H, P2, a gate on closing R5)

| place | change |
|---|---|
| bao `kv/agents/cchv-client-readers/*` | readable by the launcher AppRole and ac only (infra's policy) |
| the launcher's bao authority (AppRole `cchv-daemon`, creds `~/.config/cchv/bao-approle`) | named here; on the never-read list |
| rendered `hub.runtime.toml` | holds **sha256 hashes** of client-read tokens, never the tokens; the hub compares hashes |
| the distiller's token cache | its own file, `~/.config/cchv/distill-client-token` (mode 600); on the never-read list |
| environment overrides (`CCHV_HUB_TOKEN`) | the distiller refuses a client-read token from the environment |
| ac's browser | **`POST /v1/login`** exchanges the pasted token for an HttpOnly, Secure, SameSite=Strict session cookie (8 h); the webapp then deletes its `localStorage` copy |

- **Closing R5** needs context's GPT denyRead to cover every row above, including browser profile
  folders.
- **Live check from a GPT-routed session:** each place is unreadable, and the hub is unreachable.

### R2-I. Rollout and rollback (D-I, P2)

| stage | what | gate |
|---|---|---|
| 1 | build, gate, code review | security's code review |
| 2 | release in observe mode; sweep dry run, then sweep | counts to security and the client manager, **per session**: by class, plus the term-only sessions the hub would leave lab (Q1, H10) |
| 3 | mint the client-read tokens (ac's row); the distiller switches tokens | read-back: the distiller reads a client session **with the client token**, and a client token sees what a lab token sees plus the client rows. No 404 is expected in observe mode |
| 4 | enforce | probe the running process's **effective mode** on the health route (`client_scope.mode: enforce`), then lab, Tailscale and blog probes get 404 or absence, and client tokens see the rows |
| 5 | blog client audience | security closes the hub half of R5 |

- **Enforcement latch.** The first start with `enforce = true` writes `client_scope_state.enforced_at`.
  After that, a config saying `enforce = false` (a stale observe-era `hub.runtime.toml` from the
  launcher's cached fallback, for example) does **not** turn it off. The hub enforces, logs an
  alert line, and the health route answers 503.
- **Turning it off** needs `cchv-hub client-scope unlatch --approved-by <who>`: audited, an
  emergency switch only.
- **Rollback is to an enforcing version.** A binary older than this change cannot enforce, so the
  rollback copy kept at stage 4 is the previous enforcing build.
- **Tests:** each stage with bao down, with an observe-era cached config, with a revoked token, and
  with an older binary (the latch survives, and an older binary is refused by the launcher's minimum
  version check once enforced).

### R2-J. The oracle: request-case registry plus differential tests (D-J, P2)

- **The registry.** `tests/client_scope_cases.rs` holds one case per registered route and method.
  A test enumerates the router and fails when a route has no case.
- **Each case runs** as:
  - a lab token;
  - the trusted Tailscale header;
  - a client token;
  - no credential;
  - a HEAD request where GET exists.
- **Search runs in every mode** (text, hybrid, journal) and across pagination.
- **Differential.** For each lab-read response, the same request runs against a copy of the DB
  where the hidden rows are physically deleted. Status, headers (`X-Total-Count` included), body,
  ids, counts, ranks and order must be equal. This catches leaks of ids, counts and ordering, not
  only of strings.
- **Positive controls.** Each case asserts that it reached its handler: the lab canary is present
  where it should be.
- **Mutation check:** remove one `Visibility` predicate, and the registry goes red.

### R2-K. P3 notes

- **Matcher spec (Astra 18).** `client_scope.rs` documents:
  - byte decoding (UTF-8 with replacement; percent-decoding once);
  - case folding (Unicode lowercase);
  - the literal-substring-first behaviour of `marker_in`;
  - path joins;
  - unknown block shapes (unreadable).

  The shared fixture file adds property tests: a random lab string plus a marker inserted at a
  random place, in a random block, is client.
- **R6 restated (Astra 8).** A session stays lab until its first marked message arrives, and that may
  be never: R6 is the same residual as R4, not "about 5 minutes".
- **Project metadata (H7, Astra 14).**
  - When the project path itself matches a marker, the project's rows, aliases, worktree main paths
    and suggestions are hidden.
  - Otherwise they are shown, with counts over visible sessions only.
- **Content terms: not at launch** (Q1, the client manager; Astra disagrees, recorded). Stage 2 counts,
  per session, the sessions the distiller protected by a term only that the hub would leave lab.
  the client manager decides again with that number.

### R3-E8. `restamp-history` on the distiller's router (code review E8, P2)

**The finding.** The hub's `restamp-history` trusts the stored flags. It checks the terms as
`lower(content::text) LIKE`, with no decoding. It inspects no `project_path` and no `model`, and it
has no version guard. It must be fixed before it is ever run. It has never run.

**What it would act on** (pg1, 2026-11-02 07:5xZ). 1062 entries have no current route stamp:

| model | generated | entries | session refs | entry days |
|---|---|---|---|---|
| GPT | before routing (10-21 10:16:59Z) | 826 | 4006 | 07-04 .. 10-20 |
| Haiku | before routing | 37 | 255 | 07-04 .. 07-14 |
| GPT | after routing | 65 | 169 | 10-18 .. 10-30 |
| Claude | after routing | 134 | 476 | 10-20 .. 10-30 |

Every entry since v0.34.0 carries a stamp, and all 75 of them are `protected`. So under enforce a
lab reader sees no journal entries at all today.

**The decision: two steps, one router.**
- **The distiller classifies.** It runs the router the distiller already uses for live traffic.
- **The hub applies.** It re-checks what may have changed since, then writes.
- The hub makes no client decision of its own for a restamp. Its `LIKE` term check is deleted.

**Step 1, classify:** `cchv-distill --restamp-history --out FILE`.
- **What it touches:** hub GETs only, no model call, no write anywhere but `FILE`.
- **It refuses to start when:**
  - the routing file fails its floor record, or loads as protect-all;
  - the hub's generation differs from `sha256(routing || floor)` computed locally;
  - the caller is not a client reader. A lab principal under enforce would see a partial
    history. The hub adds `caller_scope` to `/v1/healthz/client-scope` for an authenticated
    request, so the distiller can check this.
- **The candidates:** every `/v1/journal/entries` row with `status = entry` and no route, paged to
  the end.
- **The checks, in order.** The first hit decides, and the entry stays hidden:
  1. **Model:** generated after routing went live by any Claude model → `was_protected`. The
     router of its day protected it. Before routing, the model says nothing, so the entry goes on
     to the next check.
  2. **Entry text:** the headline, the summary, each topic and each open question, normalized, with
     `marker_in` or `term_in` → `entry_text`.
  3. **Project path:** `marker_in(normalize(project_path))` → `path`.
  4. **Sessions:** `route_group`'s whole-session pass. It reads every message of every session,
     all days, sidechains included. It applies `flatten` and `normalize` (URL-decoding,
     backslashes, case) and checks `cwd`, tool values, text markers and content terms. An
     unknown shape, a failed or partial read, or a limit gives `unreadable`. A missing `cwd` gives
     `cwd_unavailable`. A session shared by many entries is read once per run.
  5. **Otherwise:** restampable.
- **The output file:**
  - It is created `O_EXCL|O_NOFOLLOW` at mode 0600.
  - **Header:** the distiller version, the full generation, `started_at`, `finished_at` and the
    hub version.
  - **One record per restampable entry:** `entry_date`, `project_path`, `generated_at` and
    `session_ids`.
  - The rest are counts per class only, so the file holds no path of a hidden entry.
- **Standard output:** counts only.

**Step 2, apply:** `cchv-hub client-scope restamp-history --from FILE --dry-run | --approved-by WHO`.
It is run by the pg1 writer.
- **It refuses the whole file when:**
  - the header's generation is not `client_scope_state.generation`;
  - the distiller version is below the version that ships this step;
  - `finished_at` is older than 24 h;
  - `--approved-by` is empty (E7);
  - `--from` is missing (the old in-hub path is gone).
- **Per listed entry**, in one transaction with its audit row (E7), it locks the row by
  `(entry_date, project_path)` and requires all of:
  - the entry still has `status = entry` and no current stamp;
  - `generated_at` equals the file's, so the entry was not regenerated since;
  - `session_ids` equal the file's;
  - every session has `client_class = 'none'` and `client_gen` equal to the current generation.
    This is an extra condition, never the only check;
  - no session has a message created after `started_at`, and no session row was updated after
    it (`sessions.updated_at`, which every ingest batch sets). The router's read must cover the
    session as it is now. (`flags_changed_at` is one global stamp, not per session, so it is not
    the check.)
- **When all of them hold,** it sets `route = 'general-restamped'` and `route_at = generated_at`. The
  audit row records the entry, the generation, `approved_by`, the file's sha256 and the distiller
  version.
- **When any of them fails,** the entry is skipped and a counter per reason goes up.
- **The dry run** makes the same checks in a read-only transaction and prints the counts.

**Order of runs.**
1. The classify run.
2. Its counts go to security and the client manager.
3. The apply dry run.
4. ac's row, with the client manager's word.
5. The apply.

None of it runs before this design is built, tested and reviewed.

**Tests.**
- **Distiller** (`scripts/test_cchv_distill.py`, fake hub). Each class stops an entry:
  - an entry written by Claude after routing;
  - a marker in the entry text;
  - a term in the entry text;
  - a URL-encoded marker in message content;
  - a backslash path;
  - a marker in `cwd`;
  - a marker in a tool input;
  - an unknown block;
  - a missing `cwd`.

  These refuse to start: a generation mismatch, a lab caller under enforce, protect-all routing,
  and an output file that already exists. The output has mode 0600 and holds paths of
  restampable entries only.
- **Hub** (`client_scope_admin_test.rs`). These refuse or skip:
  - a generation mismatch, an old distiller, a stale file, an empty `--approved-by`, and no
    `--from` (whole file);
  - a regenerated entry, a session that grew after `started_at`, and a session flagged client
    (that entry only).

  An audit failure rolls back the route write, and the dry run writes nothing.
- **Mutation evidence:** drop any single check, and a test fails.

**Security's check (2026-11-02, OK to build):**
- **E8-a, as designed:** Claude entries written after routing stay hidden. The July Haiku entries
  and the GPT entries from before routing go through the router. An entry with no model is
  before-routing evidence: it goes on to the router, never straight to restampable.
- **E8-b:** 24 h is fine. The per-entry checks for new messages and row changes carry the real
  weight.
- **E8-c:** acceptable when the file lives under `~/.config/cchv` or `~/.local/state/cchv` (both
  GPT-denied; the classify step refuses any other folder), and it is trashed after the apply or
  after 24 h, whichever comes first.

**As built** (branch `feat/cs-before-enforce`): the distiller's `--restamp-history FILE`, the hub's
`restamp-history --from FILE`, and `caller_scope` in the client-scope health. The minimum distiller
version is 0.40.0, the release that ships the classify step. Each check was removed in turn, and a
test failed each time.

**Security's code review (2026-11-02, `docs/reviews/2026-11-02-cchv-e8-restamp-code.md`), as fixed:**
- **R1:** the dry run takes no locks, so it runs on the CLI's read-only pool.
- **R2:** the accepted generation is read per entry (held `FOR SHARE` in the apply). The run stops on a
  change.
- **R3:** any route at all, stale or current, counts as a stamp.
- **R4:** the router and the generation come from the same bytes (`load_routing_snapshot`).
- **R5:** in the shared session reader, which live routing also uses, a missing, unparsable or moving
  `X-Total-Count` is an incomplete read, so the session is unreadable.
- **R6:** the apply re-checks the Claude cutoff, the entry text and the project path against the hub's
  own routing files (markers and terms). The file must carry that policy's generation.
- **R7:** the apply holds the sessions `FOR SHARE`. A row or message change since `started_at`, less
  a 15-minute margin for one ingest batch, skips the entry.
- **R8:** an entry's sessions are routed as one group, so the live per-group limits hold.
- **R9** (unknown blocks and a null `cwd` in the shared router) lands with E1, before any run.
- **P3s done:** the classify step reads as a client reader in every mode. The apply reads only a
  private regular file and refuses links. Tests cover a session-row-only change, an audit failure, and
  the dry run on a read-only pool.
- **P3s open:** the hub's effective class at classify, a producer-to-CLI test at 0.40.0, and an
  interrupted-run test. An interrupted run leaves a file that is not JSON, and the apply refuses it.
- **Residual, in the backlog:** re-check the general history when markers or terms are added.

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
4. **Claude-only client sessions that need archive search** (the client manager's fleet, through
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
| 2 | release in `observe` mode: flags computed, nothing hidden; infra runs `client-scope sweep --dry-run`, then the sweep | security reads the counts by class (no names, to the client manager too) |
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
  example cchv's own sessions answering the client manager). This is acceptable, and it is fail-closed.
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

The client manager, 2026-11-02 00:10Z (bell 312663ad), for the client side:

- **Q1 (terms):** security's call. The client manager's view: path markers only at launch, and R4
  accepted while the distiller keeps its own term check. Revisit with the observe-mode counts.
- **Q2 (totals):** counts-only totals are fine for lab readers.
- **Q3 (pre-routing entries):** hiding them by the session rule is enough, with no redaction. ac
  already decided to keep those rows (10-29), and a redaction would reopen that decision.
- **Q4 (narrowing):** for a client session, ac approves through the client desk's row. Never a lab
  session, a lab token or a lab desk row alone.
- **D5:** OK that `blog-client` (its Claude-only writer) is a client reader.
- **D7:** the observe-mode sweep counts go to the client manager, as written.
