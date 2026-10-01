## 1. Hub

- [x] 1.1 Failing integration test in `crates/hub/tests/read_test.rs`: one session, N conversation rows + M content-NULL rows (≥1 `attachment`) → `conversation_count == N` (the test may pin the fixture's `message_count`, marked as fixture-set, not a row-count guarantee)
- [x] 1.2 `SessionRow.conversation_count` + per-row `content IS NOT NULL` count in `list_sessions`; document `message_count` as a stored counter that can exceed rows (#47); regenerate `.sqlx`
- [x] 1.3 Mutation check: drop the `content IS NOT NULL` predicate, see 1.1 fail, restore, `touch`

## 2. Webapp

- [x] 2.1 `SessionRow` type in `src/services/hubApi.ts` and `src/types/archive.ts`
- [x] 2.2 Session row headline = `conversation_count`, explanatory tooltip (no number); i18n in en/ko/ja/zh-CN/zh-TW
- [x] 2.3 vitest: row renders `conversation_count`, not `message_count`

## 3. Docs

- [x] 3.1 `docs/archive/` paragraph: message_count ≠ conversation items (#47); why the journal skips ~half its days

## 4. Gate

- [x] 4.1 `just archive-test`, `just archive-lint`, `cargo fmt --all -- --check`, `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`, `pnpm run i18n:validate`
- [x] 4.2 the AGENTS.md internal-hostname grep shows nothing new (origin is a public fork); push to `internal` only
