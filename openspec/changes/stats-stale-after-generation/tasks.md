## 1. Tests first

- [x] 1.1 Daily: adopt a fake generation with no sweep, a lab session written before the margin -> 503 naming the count and generation (red on main).
- [x] 1.2 Daily: after a sweep stamps it -> 200 with the session counted, `unscanned_excluded = 0`.
- [x] 1.3 Daily: the only unscanned lab session written inside the margin -> 200, `unscanned_excluded = 1`.
- [x] 1.4 Daily: the only unscanned session is client-class -> 200, `unscanned_excluded = 0`.
- [x] 1.5 Daily: a full-scope reader -> 200 with the session, `unscanned_excluded = 0`.
- [x] 1.6 Tool-calls: the 503 case and the inside-margin case.

## 2. Build

- [x] 2.1 Shared check in `stats_api.rs` (D1-D6), used by `daily` and `tool_calls`; `unscanned_excluded` on both answers.
- [x] 2.2 Docs: `docs/archive/deployment.md` stats routes (503 during a generation change; the field).

## 3. Ship

- [ ] 3.1 Gate, security review (diff), release, swap via infra; tell monitoring and quota-tap about the 503 and the field.
