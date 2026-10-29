## 1. Daemon

- [x] 1.1 `PathExclude` (parse fails closed, `excludes`), unit tests.
- [x] 1.2 `project_path_exclude` in the config; start-up count log.
- [x] 1.3 `run_once_scoped` skips matching projects before parsing; `projects_path_excluded` in the stats.
- [x] 1.4 Integration test: the canary project is never sent, the control is.

## 2. Release and ops

- [x] 2.1 Linux build for the guest; infra wires the option and runs the canary (a session under the excluded path never reaches the hub, a control does). PASS 2026-10-28 16:51Z on agent2 (build 80088684): canary sessions 0 in the hub (both attempts), controls present, 0 projects under the excluded path; daemon reported projects_path_excluded 1.
- [x] 2.3 `HUB_TOKEN` fills a `hub_token` the config file leaves out (infra's request after the canary); test.
- [x] 2.2 Merge and release on manager-lab's go.

Released in cchv-v0.30.0 (tag 64714d42); the Linux sync-daemon asset is on that release, and rolling it to the guests is infra's.
