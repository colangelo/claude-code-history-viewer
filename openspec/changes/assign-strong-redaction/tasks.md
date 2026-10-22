## 1. Engine

- [x] 1.1 `Rule::AssignStrong` (`assign-strong`), derived from an `assign` hit with the strong shape; `Rule::ALL` and `from_id` include it.
- [x] 1.2 Selecting `assign` covers `assign-strong` (redaction, field grouping, dry-run filters).
- [x] 1.3 Fixed marker `[REDACTED:assign-strong]`, both in text and in whole-value JSON replacement.
- [x] 1.4 `messages_by_rule` in the findings summary and health body, excluding `api-request-blob`.
- [x] 1.5 Tests: strong value redacted with the fixed marker; controls untouched (a 40-char hex sha, a short `TOKEN=abc123`); `assign` selection still covers strong; the health count excludes copies.

## 2. Release and ops

- [ ] 2.1 Release; the hub swap and the `hub.toml` edit in one step; read back with a planted fake on the test DB plus the two controls.
- [ ] 2.2 One bell to security and manager-lab when live, with the first day's count.
