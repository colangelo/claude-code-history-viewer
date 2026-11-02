# Tasks: hub client-read scope

Nothing below starts until security's two-vendor review of `design.md` is OK (stage 0).

## 1. Matcher (D2)

- [ ] 1.1 Write `scripts/fixtures/client-scope-cases.json`, made-up markers only:
      - normalisation: URL-encoded, backslash, case;
      - segment boundaries: punctuation and wrappers match, longer names do not, `.git` matches;
      - classes: path, cwd, tool_input, text_marker, unreadable, none.
- [ ] 1.2 Red: hub unit tests and the distiller's test both run the fixture. The hub side fails.
- [ ] 1.3 Green: `crates/hub/src/client_scope.rs`, made of `normalize`, `Markers::load` (with the
      floor check) and `classify_message`.
- [ ] 1.4 Mutation check: drop the segment rule, and drop the decode. Both suites go red.

## 2. Storage and flag (D1, D3, D4)

- [ ] 2.1 Migration:
      - `sessions.client_class TEXT NULL` plus `client_scanned_sha TEXT NULL`;
      - table `client_scope_state` (routing_sha, swept_at);
      - table `client_scope_clears` (session, class, approved_by, at).
- [ ] 2.2 Red, then green: ingest sets the class in the batch transaction. The flag is sticky: a
      later `none` never clears a client class.
- [ ] 2.3 Red, then green: CLI `client-scope sweep [--dry-run] [--batch N]`. It prints counts by
      class only, and it is re-entrant.
- [ ] 2.4 Red, then green: CLI `client-scope clear --session <id> --approved-by <who>`. It is
      audited and refuses without an approval.
- [ ] 2.5 Fail-closed tests:
      - a missing routing file, one below floor, or one whose hash changed puts the hub in
        `unknown`, and every session counts as client under enforce;
      - an unscanned session counts as client;
      - `/v1/healthz/client-scope` answers 503 when unknown.

## 3. Scopes and routes (D5, D6)

- [ ] 3.1 Config:
      - `client_read` on `[[tokens]]` and `[[readers]]` (`deny_unknown_fields` kept);
      - `[client_scope] routing_file`, `floor_file`, `enforce`.
- [ ] 3.2 `Reader { principal, scope }` replaces `Authenticated`, and the Tailscale identity is
      always `Lab`. Each statement takes `Visibility` with no default, and the predicate helper
      builds the filter.
- [ ] 3.3 Every route in the D6 table, each with a test for its lab-read behaviour.
- [ ] 3.4 Route-walk canary test over every registered GET route. Mutation: remove one predicate,
      and the test goes red.
- [ ] 3.5 Alias writes: a lab-read caller naming a client project gets 404.

## 4. Consumers

- [ ] 4.1 Launcher: render `kv/agents/cchv-client-readers/*` with `client_read = true`; any
      duplicate is fatal. Extend the e2e harness.
- [ ] 4.2 Distiller:
      - reads with the client-read token and writes with the machine token;
      - `route_group` asks the hub's class first;
      - the fixture test from 1.2 runs on the distiller side.
- [ ] 4.3 Docs: `docs/archive/deployment.md` (scopes, the bao paths, enforce and rollback).

## 5. Rollout (D7)

- [ ] 5.1 Gate green, then send security the code review. Release in observe mode.
- [ ] 5.2 infra runs the sweep dry run, then the sweep. Counts go to security and manager-aruba.
- [ ] 5.3 Tokens minted on ac's row; the distiller switches; read-back.
- [ ] 5.4 `enforce = true`, then the probes from D7 stage 4.
- [ ] 5.5 Blog client audience on its own token (context). Security closes the hub half of R5.
