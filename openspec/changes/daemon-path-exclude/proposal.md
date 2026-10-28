## Why

A shared guest machine may hold client-data checkouts in its home directory (ac/claude-code-history-viewer#84). The
sync daemon can skip whole providers only (`providers_exclude`), so every session run
in such a directory would be uploaded to the hub. A hub-side rule would drop it only
after the data had already left the guest.

## What Changes

- New daemon option `project_path_exclude`: shell globs on a project's decoded path
  (the directory the agent ran in), e.g. `["/home/agent/client*"]`. `*` also crosses
  `/`, so a pattern covers the tree below a matching directory.
- A matching project is skipped before any of its sessions is parsed or uploaded, and is
  not recorded in the checkpoint. Each pass counts skipped projects in
  `projects_path_excluded`.
- A config file may leave `hub_token` out; `HUB_TOKEN` from the environment fills it, so a
  guest that reads its token from the vault at each start never writes it to disk. A value
  in the file wins. With neither, the daemon refuses to start.
- A pattern that does not parse stops the daemon at start-up, so a typo cannot upload what
  it was meant to keep back. The start-up log states how many patterns are active.

## Impact

- `crates/sync-daemon` only: config, a `path_exclude` module, `sync::run_once_scoped`
  (`run_once` keeps its signature and excludes nothing). New dependency `glob` (already
  in the lock file).
- A daemon release; the guest wires the option into its config with infra's canary.
- Existing configs are unchanged: the option defaults to empty.
