## Why

The daemon reads Claude Code sessions from one folder only, `~/.claude` (on our Macs a
symlink to `~/.config/claude`), for both the scan and the file watcher. A Claude Code
session started with a different `CLAUDE_CONFIG_DIR` writes its transcripts somewhere
else, and none of them reach the hub. The first case: a helper session on one Mac moves
to its own account folder, `~/.config/claude-hlp-b`, with its own `projects/`.

Discovering every `~/.config/claude-*/` folder automatically was considered and rejected:
on the always-on Mac that pattern also matches a January backup of the main folder, a
test folder and an unrelated account, so a binary swap would silently start uploading
duplicate and unwanted sessions. Each extra folder is named in the config instead.

## What Changes

- New daemon option `claude_extra_roots`: a list of Claude config folders (each one holds a
  `projects/`), e.g. `["~/.config/claude-hlp-b"]`. A leading `~/` is the home directory.
- Each listed root is scanned exactly like `~/.claude`: provider `claude`, the same project
  path decoding, so a repo used from both folders lands in one hub project. Each root is
  also a file-watcher root.
- Roots are canonicalized at start-up. A root that does not exist, is not absolute after
  `~` expansion, or resolves to the main folder or to a root already listed is skipped
  with a warning, so the same transcripts are never read twice and a missing folder never
  stops the daemon. The start-up log names the active extra roots.
- `providers_exclude = ["claude"]` skips the extra roots too, and `project_path_exclude`
  applies to their projects as to any other.

## Impact

- `crates/sync-daemon` only: config, a `claude_roots` module, `sync::run_once_scoped`
  takes the resolved roots (`run_once` keeps its signature and adds none). No hub change,
  no migration.
- A daemon release, then one config line on the Mac whose helper moved. Checkpoints are
  per file, so the first pass after the install sends every session already in the folder.
- Existing configs are unchanged: the option defaults to empty.
