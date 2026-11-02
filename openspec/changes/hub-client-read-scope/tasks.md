# Tasks: hub client-read scope (Revision 2)

Security accepted design Revision 2 (7cd5a699) on 2026-11-02, with additions to R2-F; the build
may start.

L1 (`/v1/healthz/journal` paths) is already fixed in v0.38.1.

## 1. Matcher and extractor (D2, R2-A, R2-K)

- [ ] 1.1 Write `scripts/fixtures/client-scope-cases.json`, made-up markers only. It covers:
      - normalisation: percent-decoding once, backslashes, Unicode case;
      - segment boundaries;
      - literal-substring-first behaviour;
      - every content block shape: text, thinking, nested tool_use and tool_result, images and
        documents, an unknown shape (unreadable);
      - `raw` leaves, metadata fields, a marker past 64 KiB.
- [ ] 1.2 Red: the hub and the distiller suites both run the fixture, plus property tests (a marker
      inserted at a random place in a random block).
- [ ] 1.3 Green: `client_scope.rs`, with `normalize`, `Markers`, `Extract` (one extractor for ingest
      and sweep: full `content`, full `raw`, session and project fields; size, depth and shape limits
      mean unreadable) and `classify`.
- [ ] 1.4 Mutation check: drop the segment rule, the decode, the tool_result walk and the `raw`
      walk. Each one turns a suite red.

## 2. Storage, policy, family (R2-B, R2-C, R2-D)

- [ ] 2.1 Migration:
      - `sessions`: `client_class`, `client_gen`, `client_scanned_upto`, `row_rev`;
      - new tables `client_scope_policy` (marker hashes, generation), `client_scope_state` (mode,
        `enforced_at`) and `client_scope_audit`.
- [ ] 2.2 Policy snapshot, re-validated every 60 s and per batch:
      - a generation change withdraws lab visibility first;
      - markers only added: accept;
      - any marker removed or replaced: `unknown` until `accept-routing`.

      Tests: delete, corrupt, extend, replace, and a same-count swap.
- [ ] 2.3 Ingest:
      - classify every batch in its own transaction, metadata-only updates included;
      - the watermark rule;
      - family propagation (copies, parent and child, orphan quarantine).

      Tests per R2-A and R2-B.
- [ ] 2.4 Sweep: `client-scope sweep [--dry-run]`. Writes are monotonic and conditional on the
      revision; it survives a crash and a resume, and two sweeps at once are safe. Its counts are
      per session, by class, plus term-only sessions the hub would leave lab (this one needs the
      terms read-only, counted, never used for the flag), plus sessions hidden only by family from a
      term-protected entry.
- [ ] 2.5 Audited CLIs: `accept-routing`, `clear`, `unlatch`, `restamp-history [--dry-run]`.
      Each refuses to run without `--approved-by`. `restamp-history` classifies the entry's own text
      (summary, threads, every served field) and its sessions with the full router, terms included,
      and restamps only when both match nothing. Its write waits for ac's row and the client manager's
      word.

## 3. Scopes and routes (D5, D6, R2-E, R2-F, R2-G)

- [ ] 3.1 Config:
      - `client_read` on token entries, carried as a **hash** in the rendered config;
      - `[client_scope] routing_file`, `floor_file`, `enforce`.
- [ ] 3.2 `Reader { principal, scope }`. The Tailscale identity is always `Lab`. `Visibility` is an
      argument with no default on every statement, and one predicate helper builds the filter.
- [ ] 3.3 Routes: search (every mode, pagination), browse, session messages, journal entries and
      threads (full-history provenance), stats (from visible rows; the mirror's class and
      generation, 503 when stale), identities and aliases, and the journal health detail.
- [ ] 3.4 One missing-resource contract. Visibility is applied before id resolution (ambiguity
      400s), and writes are authorized before validation.
- [ ] 3.5 Request-case registry (fails when a route has no case), differential tests (hidden rows
      deleted), HEAD, the Tailscale header, positive controls. Mutation: remove one predicate, and
      it goes red.
- [ ] 3.6 `POST /v1/login`: the HttpOnly cookie exchange for a client-read token; the webapp drops
      its `localStorage` copy after it.

## 4. Consumers and enforcement latch (R2-H, R2-I)

- [ ] 4.1 Launcher: render the `cchv-client-readers` children as hashes with `client_read = true`;
      duplicates are fatal; a minimum hub version once `enforced_at` is set. Extend the e2e
      harness: bao down, an observe-era cache, a revoked token.
- [ ] 4.2 The enforcement latch, and a health field `client_scope.mode`.
- [ ] 4.3 Distiller:
      - its own client token file;
      - it refuses a client token from the environment;
      - it reads the hub's class first;
      - the fixture test.
- [ ] 4.4 Docs: `deployment.md`, covering scopes, where credentials rest (the R2-H table), the
      latch, rollback to an enforcing version, and the audited CLIs.

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
