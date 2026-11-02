# Tasks: hub client-read scope (Revision 2)

Security accepted design Revision 2 (7cd5a699) on 2026-11-02, with additions to R2-F; the build
may start.

L1 (`/v1/healthz/journal` paths) is already fixed in v0.38.1.

## 1. Matcher and extractor (D2, R2-A, R2-K)

- [x] 1.1 Write `scripts/fixtures/client-scope-cases.json`, made-up markers only. It covers:
      - normalisation: percent-decoding once, backslashes, Unicode case;
      - segment boundaries;
      - literal-substring-first behaviour;
      - every content block shape: text, thinking, nested tool_use and tool_result, images and
        documents, an unknown shape (unreadable);
      - `raw` leaves, metadata fields, a marker past 64 KiB.
- [x] 1.2 Red: the hub and the distiller suites both run the fixture. (Property tests: NOT yet;
      the 48 fixture cases and 4 mutations stand in for now.)
- [x] 1.3 Green: `client_scope.rs`, with `normalize`, `Markers`, `Extract` (one extractor for ingest
      and sweep: full `content`, full `raw`, session and project fields; size, depth and shape limits
      mean unreadable) and `classify`.
- [x] 1.4 Mutation check: drop the segment rule, the decode, the tool_result walk and the `raw`
      walk. Each one turns a suite red.

## 2. Storage, policy, family (R2-B, R2-C, R2-D)

- [x] 2.1 Migration:
      - `sessions`: `client_class`, `client_gen`, `client_scanned_upto`, `row_rev`;
      - new tables `client_scope_policy` (marker hashes, generation), `client_scope_state` (mode,
        `enforced_at`) and `client_scope_audit`.
- [x] 2.2 Policy snapshot, re-validated every 60 s and per batch:
      - a generation change withdraws lab visibility first;
      - markers only added: accept;
      - any marker removed or replaced: `unknown` until `accept-routing`.

      Tests: delete, corrupt, extend, replace, and a same-count swap.
- [x] 2.3 Ingest:
      - classify every batch in its own transaction, metadata-only updates included;
      - the watermark rule;
      - family propagation (copies, parent and child, orphan quarantine).

      Tests per R2-A and R2-B.
- [x] 2.4 Sweep: `client-scope sweep [--dry-run]`. Writes are monotonic and conditional on the
      revision; it survives a crash and a resume, and two sweeps at once are safe. Its counts are
      per session, by class, plus term-only sessions the hub would leave lab (this one needs the
      terms read-only, counted, never used for the flag), plus sessions hidden only by family from a
      term-protected entry.
- [x] 2.5 Audited CLIs: `accept-routing`, `clear`, `unlatch`, `restamp-history [--dry-run]`.
      Each refuses to run without `--approved-by`. `restamp-history` classifies the entry's own text
      (summary, threads, every served field) and its sessions with the full router, terms included,
      and restamps only when both match nothing. Its write waits for ac's row and the client manager's
      word.

## 3. Scopes and routes (D5, D6, R2-E, R2-F, R2-G)

- [x] 3.1 Config:
      - `client_read` on token entries, carried as a **hash** in the rendered config;
      - `[client_scope] routing_file`, `floor_file`, `enforce`.
- [x] 3.2 `Reader { principal, scope }`. The Tailscale identity is always `Lab`. `Visibility` is an
      argument with no default on every statement, and one predicate helper builds the filter.
- [x] 3.3 Routes: search (every mode, pagination), browse, session messages, journal entries and
      threads (full-history provenance), stats (from visible rows; the mirror's class and
      generation, 503 when stale), identities and aliases, and the journal health detail.
- [x] 3.4 One missing-resource contract. Visibility is applied before id resolution (ambiguity
      400s), and writes are authorized before validation.
- [x] 3.5 Request-case registry (fails when a route has no case), differential tests (hidden rows
      deleted), HEAD, the Tailscale header, positive controls. Mutation: remove one predicate, and
      it goes red.
- [x] 3.6 `POST /v1/login`: the HttpOnly cookie exchange (hub side done and tested). The webapp
      side (use it, drop the `localStorage` copy) is NOT done: it is a visible UI change and goes
      through a frontend review first.

## 4. Consumers and enforcement latch (R2-H, R2-I)

- [x] 4.1 Launcher: render the `cchv-client-readers` children as `[[client_readers]]` hashes;
      duplicates are fatal (e2e with a fake bao). The minimum hub version once `enforced_at` is set
      is NOT done (the latch itself is in the hub). Extend the e2e
      harness: bao down, an observe-era cache, a revoked token.
- [x] 4.2 The enforcement latch, and a health field `client_scope.mode`.
- [~] 4.3 Distiller (own token file and env refusal DONE; reading the hub's class first NOT done,
      an optimization only: both checks only add protection):
      - its own client token file;
      - it refuses a client token from the environment;
      - it reads the hub's class first;
      - the fixture test.
- [x] 4.4 Docs: `deployment.md`, covering scopes, where credentials rest (the R2-H table), the
      latch, rollback to an enforcing version, and the audited CLIs.

## 4b. Before enforce (security code review, 2026-11-02, security ddff6c0)

Observe mode may ship once security re-checks O1-O3 and the small items (done:
98b2573e, 020ddf5f, 06da3802, 0f040d36, 565d3ae5). Nothing below may be skipped
before `enforce = true`:

- [ ] E1 Extractor coverage:
      - classify project `name`, git remote, worktree main path and project-only updates;
      - derive or validate `search_text`;
      - the distiller's path plus pattern join;
      - an unknown block type is unreadable (and the fixture says so);
      - tool-block top-level keys;
      - the 16 MiB budget per message, not per field.
- [ ] E2 Family:
      - `raw` parent links and orphan quarantine;
      - a fixed-point closure (siblings, grandchildren, the copies of protected-entry sessions) over
        stored family edges, under a family lock;
      - a message-only batch keeps `file_path`;
      - the sweep propagates in the same transaction.
- [ ] E3 The distiller reads the hub's class before ANY general call: a family-only or raw-only
      client session never goes to the general model.
- [ ] E4 Policy refresh:
      - withdraw lab visibility before the audit write;
      - serialize refreshes;
      - compare-and-swap the accepted set in one transaction.
- [ ] E5 Authorize, then fetch: visibility on the actual fetch (message browse, semantic and hybrid
      fetch-by-id, stats); the mirror copies messages and sessions under one snapshot.
- [ ] E6 Identity metadata from visible rows only: display name, alias targets, expansion,
      suggestions, `last_modified` ordering.
- [ ] E7 Admin audit:
      - one transaction with the full evidence (removed hashes, generation);
      - the restamp CLI refuses an empty `--approved-by`.
- [ ] E8 restamp-history uses the distiller's FULL router (decoding, project path, model, version
      guard), before it is ever run. Design: `design.md` § R3-E8, with security's check first.
      - [x] hub: `caller_scope` in `/v1/healthz/client-scope` for an authenticated request;
      - [x] distiller: `--restamp-history FILE` (refusals, the checks in order, a session cache, a
            0600 `O_EXCL|O_NOFOLLOW` file under a GPT-denied folder, counts on stdout), with tests;
      - [x] hub: `restamp-history --from FILE` (refusals for the whole file, a per-entry re-check
            and write in one transaction with its audit), the `LIKE` path deleted, with tests;
      - [x] mutation evidence for every check (distiller 11, hub 10);
      - [ ] security's code review, then release 0.40.0; the runs follow the order in R3-E8.
- [ ] E9 Credentials:
      - no AppRole secret or bao token in `curl` argv (use stdin or a file descriptor);
      - the client cache is created `O_EXCL|O_NOFOLLOW` at mode 0600.
- [ ] E10 The mirror records its generation; lab stats compare it as well as `flags_changed_at`.
- [ ] E11 The oracle:
      - method-aware registry, HEAD, a machine-token walk;
      - stats with a ready mirror and a positive control;
      - an embedder stub for semantic and hybrid;
      - differential over stats, health, pagination, and shared-project and identity cases;
      - mutation evidence per predicate;
      - property tests.
- [ ] E12 Threads: protection over the full history, the state from visible inputs only.
- [ ] E13 Revocation without a restart (credential epochs or mandatory reload); a cached render never
      resurrects a revoked token.
- [ ] E14 The cookie: HTTPS listener only, named `__Host-`, and a test that no response carries
      `Access-Control-Allow-Credentials`.
- [ ] E15 The launcher's minimum hub version once latched.
- [ ] One definition of "general" (`general` and `general-restamped`), used by every filter.

## 5. Rollout (R2-I)

- [ ] 5.1 Gate green, then security's code review, then release in observe mode.
- [ ] 5.2 infra runs the sweep dry run, then the sweep. Per-session counts go to security and
      the client manager, and the client manager re-decides on terms. infra also runs the `restamp-history`
      dry run, counts to security and the client manager.
- [ ] 5.3 Tokens minted on ac's row; the distiller switches; read-back (no 404 expected).
- [ ] 5.4 Enforce: probe the effective mode, then the lab, Tailscale and blog probes, and the
      client-token probes.
- [ ] 5.5 Blog client audience (context). GPT denyRead covers every R2-H place, with a live check
      from a GPT session. Security closes the hub half of R5.
