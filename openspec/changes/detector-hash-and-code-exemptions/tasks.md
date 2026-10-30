## 1. Detector

- [x] 1.1 `password_hash` (bcrypt, argon2, sha-crypt; anchored) in `assignment_finding` and pair's value floor; test with a must-fire tampered control.
- [x] 1.2 `closed_literal_code` for quoted assign captures (leading `) ] } ,` plus whitespace or another bracket); test with a must-fire `)`-leading secret and a lone leading comma.

## 2. Release

- [ ] 2.1 Release with v0.32.0 (with `daemon-claude-extra-roots`) on ac's word; swap via infra.
- [ ] 2.2 After the swap, a live canary: a planted bcrypt hash and the parse line are kept, a `)`-leading fake is redacted; ack the canary's findings at once.
