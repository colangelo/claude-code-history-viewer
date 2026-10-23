## 1. Engine

- [x] 1.1 `Rule::AssignStrong` (`assign-strong`), derived from an `assign` hit with the strong shape; `Rule::ALL` and `from_id` include it.
- [x] 1.2 Selecting `assign` covers `assign-strong` (redaction, field grouping, dry-run filters).
- [x] 1.3 Fixed marker `[REDACTED:assign-strong]`, both in text and in whole-value JSON replacement.
- [x] 1.4 `messages_by_rule` in the findings summary and health body, excluding `api-request-blob`.
- [x] 1.5 Tests: strong value redacted with the fixed marker; controls untouched (a 40-char hex sha, a short `TOKEN=abc123`); `assign` selection still covers strong; the health count excludes copies.

## 2. Release and ops

- [x] 2.1 Release; the hub swap and the `hub.toml` edit in one step; read back with a planted fake on the test DB plus the two controls. Live 2026-10-22T08:59:00Z (infra-disk); the live canary through `/v1/ingest` also passed (strong fake stored as the fixed marker, both controls byte-identical).
- [x] 2.2 One bell to security and manager-lab when live, with the first day's count. (2026-10-23 09:07Z: real ingest assign 65 msgs, assign-strong 3, pair 0, prefix 1; canaries and the 2026-10-22 sweep excluded.)

## 3. Inventory flags in the dry run (manager-lab GO, 2026-10-22)

- [x] 3.1 `redact::is_reference` (`${`, `$(`, `%VAR%`, `{{`, `#{`) and `redact::is_placeholder` (`xxx`, `changeme`, `example`, `dummy`, `redacted`, `<…>`, `***`, case-insensitive), shared with the v0.28.1 exemption. A whole-value `${VAR}` never reaches a rule (the existing placeholder guard); the flag covers a reference inside a longer value.
- [x] 3.2 `--fingerprint-key-file` hits also carry `ref`, `ph`, the source kind (`tool_result`, `tool_input`, `text`, `other`, `search_text`) from the JSON walk, and for a tool result the basename of the file the producing call read (a `Read`, or `cat`/`grep`/`sed` in `Bash`), resolved per session by `tool_use_id`. Never a path; Claude transcripts only.
- [x] 3.3 Tests: flag fixtures built from parts, the origin walk, basename extraction, and an integration test linking a tool result to its `Read`.
- [x] 3.4 Read-only run from a branch build (2026-10-22, 09:32-09:43Z); per-fingerprint detail to the client-data owner only, counts to manager-lab and security.
