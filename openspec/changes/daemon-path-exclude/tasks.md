## 1. Daemon

- [x] 1.1 `PathExclude` (parse fails closed, `excludes`), unit tests.
- [x] 1.2 `project_path_exclude` in the config; start-up count log.
- [x] 1.3 `run_once_scoped` skips matching projects before parsing; `projects_path_excluded` in the stats.
- [x] 1.4 Integration test: the canary project is never sent, the control is.

## 2. Release and ops

- [ ] 2.1 Linux build for the guest; infra wires the option and runs the canary (a session under the excluded path never reaches the hub, a control does).
- [ ] 2.2 Merge and release on manager-lab's go.
