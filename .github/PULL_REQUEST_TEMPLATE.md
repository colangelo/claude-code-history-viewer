<!--
Thanks for contributing to claude-code-history-viewer! 🙏
Please target the `develop` branch (NOT `main` — main is release-only).
-->

## What & why

<!-- What does this change and why? Link the issue it closes. -->

Closes #

## Type

- [ ] Bug fix
- [ ] New feature
- [ ] New provider (Claude Code / Codex / OpenCode / Kiro / Kimi / Copilot / …)
- [ ] Refactor / perf
- [ ] Docs

## Checklist

- [ ] PR targets **`develop`**, not `main`
- [ ] Added/updated **tests** for the changed behavior (`pnpm vitest run`, and Rust tests if backend)
- [ ] `pnpm exec tsc --build .` and `pnpm lint` pass
- [ ] **i18n**: any new user-facing string is `t()`-wrapped and the key exists in **all 5 languages** (`en, ko, ja, zh-CN, zh-TW`) — verified with `pnpm run i18n:validate`

## If this adds a frontend-callable backend command

- [ ] Routed in the Axum WebUI router (`crates/viewer/src/server/mod.rs`) with a handler in `server/handlers.rs`, and classified read-only or mutating there — otherwise `--serve` answers 404/405

## If this adds a new provider

- [ ] Followed the existing provider pattern in `crates/viewer/src/providers/`
- [ ] Session discovery verified on macOS / Linux / Windows (and WSL if applicable)
- [ ] No leftover identifiers copied from another provider (names, comments, fixtures)

<!--
Note: a maintainer (and possibly the @claude bot) will review. You can mention
@claude in a comment to ask for an automated i18n/parity check.
-->
