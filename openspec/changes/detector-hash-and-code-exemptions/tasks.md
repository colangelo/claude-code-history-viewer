## 1. Detector

- [x] 1.1 `password_hash` (bcrypt, argon2, sha-crypt; anchored) in `assignment_finding` and pair's value floor; test with a must-fire tampered control.
- [x] 1.2 `closed_literal_code` for quoted assign captures (leading `) ] } ,` plus whitespace or another bracket); test with a must-fire `)`-leading secret and a lone leading comma.

## 2. Release

- [x] 2.1 Release with v0.32.0 (with `daemon-claude-extra-roots`) on ac's word; swap via infra.
- [x] 2.2 After the swap, a live canary: a planted bcrypt hash and the parse line are kept, a `)`-leading fake is redacted; ack the canary's findings at once.

Released in cchv-v0.32.0 (tag 8c5aa337) on ac's yes to a1030-08; live on m4m 2026-10-30 10:25:29Z. Live canary cchv-canary-0.32.0-517b0e5e PASS (bcrypt, sha-crypt and the code line kept; a )-leading fake redacted); the same canary failed the three keep cases on 0.31.0.
