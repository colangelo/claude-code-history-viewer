## 1. Classifier

- [x] 1.1 Tests first (`client_scope.rs`): a registered block is subtracted; one changed byte still
      marks; text appended after the block marks; an attachment with a second, client file raises; a
      block not at a line start marks; the same block in `search_text` and `raw` is subtracted.
- [x] 1.2 `InstructionBlocks` (head index, line-start search) and `Markers::with_instructions`; the
      message extractor matches the pieces outside registered blocks.

## 2. Policy

- [x] 2.1 Tests first (`client_policy.rs`): entries parse; short, malformed or non-hex entries make
      the policy unknown; an added block is a narrowing, a removed block a widening.
- [x] 2.2 Parse `[protected].instruction_blocks`; `Policy::accepted_set` with `instruction:` rows;
      `compare` reads the two kinds apart; `client_state` names the narrowing.

## 3. CLI

- [x] 3.1 `hub client-scope instruction-entry FILE`: prints the TOML entry for the trimmed file.
- [x] 3.2 `hub client-scope census --routing FILE --floor FILE [--days N] [--ids]`: read-only,
      counts by stamped → candidate class, ids that would be `none`. Test against the test database.

## 4. Review and rollout

- [x] 4.1 Hub suite, clippy, fmt; `docs/archive/deployment.md` § client scope: registering a block.
- [x] 4.2 Diff to security (two-vendor review): merge after U1-U7 (ac/security af08b2f).
- [x] 4.2a U1-U7 and T1: sourced entries verified at accept, no block bootstrap, conservative guard
      (differential test), union of occurrences, edge check, strict hex, exact cwd; end-to-end accept,
      sweep and clear test; census on a read-only connection.
- [ ] 4.2b Single-reader re-check by security.
- [ ] 4.3 Census on pg1 with a candidate routing file (the six global-file versions); counts and ids to
      manager-aruba.
- [ ] 4.4 Release; manager-aruba registers the blocks; infra runs `accept-routing` right after.
