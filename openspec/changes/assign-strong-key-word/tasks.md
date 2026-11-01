## 1. Tests first (red)

- [x] 1.1 `redact.rs` unit tests: a 32-char alphanumeric `NATS_PASSWORD` is `assign-strong` and redacted; `apiKey` in JSON; `GITHUB_TOKEN`, `db_passwd`, `client_secret` word-final keys
- [x] 1.2 Controls: `password_reset_url` with a 3-class value stays `assign`; a 40-char hex digest under `NATS_PASSWORD` stays; a call and a dotted path under `ACCESS_TOKEN` stay; a reference (`${…}`) never strong; 31 chars stays
- [x] 1.3 The accepted cost pinned: a bare snake_case identifier with a digit (3 classes, 32+) under `ACCESS_TOKEN` is redacted
- [x] 1.4 Run them, record red: 3 failed (the positive cases), controls pass, 2026-11-01

## 2. Code (green)

- [ ] 2.1 `strong()`: the 32-63 band accepts 3 classes when the key ends in a strong word
- [ ] 2.2 Full hub tests, clippy, fmt

## 3. Release

- [ ] 3.1 Ships in the next cchv release, on ac's word
- [ ] 3.2 After the swap: the hub CLI dry run on pg1 (`hub findings dry-run --rule assign-strong`), count to security
- [ ] 3.3 The backfill sweep of stored rows: to manager-lab for a go (a prod archive write), run by the pg1 writer
