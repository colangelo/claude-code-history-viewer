# Tasks

- [x] 1.1 Tests first (red): positive/negative controls for `assign-hex-secret`; placeholder words;
      the volume count with 10 unacked / 10 acked false_positive / 10 acked handled; #76 test's
      `messages_by_rule_all` expectation updated to the ruling (handled still counts).
- [x] 1.2 `Rule::AssignHexSecret` (id `assign-hex-secret`, in ALL, selected by `assign`).
- [x] 1.3 `assignment_finding`: hex branch after strong; keys ending BEARER / API-KEY / PASSW
      admitted (added to the assign key pattern; plain `assign` stays gated by `secret_key`). Two old
      assign-strong controls (`COMMIT_TOKEN=`/`NATS_PASSWORD=` + 40 hex) now report the new rule.
      Red b5f16521; mutation-checked: key contains vs ends, volume drops handled, no placeholder
      words — each fails a test.
- [x] 1.4 `placeholder`: ALL_CAPS GENERATE_/EXAMPLE_/YOUR_/CHANGE_ME word.
- [x] 1.5 findings summary: `messages_by_rule_all` from its own query (exclude false_positive only).
- [x] 1.6 DROPPED: the marker re-quote shapes I could reconstruct already pass (a pin test stays);
      the live hits' exact text is not reproducible from the redacted rows, so no change without a
      failing case.
- [ ] 1.7 Gate; security review; release; infra runs the dry-run count; security says redact or not.
