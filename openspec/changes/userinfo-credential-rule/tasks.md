## 1. Detector

- [x] 1.1 Tests first (`redact.rs` tests): rule id, store-line redaction, flag-only, DB URL / JSON / wrapped host, negative controls, overlap with `prefix`.
- [x] 1.2 `Rule::Userinfo`, pattern, `userinfo_placeholder`, `Rule::ALL`/`RULES`/`PATTERNS` arity.
- [x] 1.3 Elision rule for display masks (found by the real 2-day dry run: 3 of 7 hits).

## 1b. Security c1afb74 rows 1-6 (0.44.2)

- [x] 1b.1 Narrow exemptions (row 1), authority parse with last `@` (row 2), token in user position (row 3), repeats within a text and across a message (row 4), query/fragment end the authority (row 5), tests and spec correction (row 6).

## 2. Hub surface

- [x] 2.1 `credential_findings_test`: `messages_by_rule_all` includes `userinfo`; new ingest test for the three fields.

## 3. Docs and gate

- [x] 3.1 `docs/archive/deployment.md`: add `userinfo` to the rule list in § Credential findings and redaction, with the flag-only default and the real-run command.
- [ ] 3.2 Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` (1.99.0), hub tests `--test-threads=1` (heavy lock), frontend gate through the commit hook.
- [ ] 3.3 Security review (new detector surface: two vendors), then release and deploy; read the live version and the rule's presence in `messages_by_rule_all`.
