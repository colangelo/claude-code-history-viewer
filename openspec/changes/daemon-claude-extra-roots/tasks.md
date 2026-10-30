## 1. Daemon

- [x] 1.1 `claude_roots::resolve` (`~` expansion, canonicalize, skip missing / relative / main / duplicate with a warning), unit tests.
- [x] 1.2 `claude_extra_roots` in the config (default empty); start-up log of the active roots; extra roots added to the watcher.
- [x] 1.3 `run_once_scoped` scans each extra root as a Claude root, unless `claude` is in `providers_exclude`.
- [x] 1.4 Integration test: a session under an extra root reaches the hub as provider `claude`; without the option it does not.

## 2. Release and ops

- [x] 2.1 Release with v0.32.0 on ac's word (asks row via manager-lab).
- [x] 2.2 ac-mbm5: mac-m5 installs the daemon and adds `claude_extra_roots = ["~/.config/claude-hlp-b"]` (via that Mac's gestore); cchv reads back the helper's sessions in the hub.

Released in cchv-v0.32.0 (tag 8c5aa337), with a new asset cchv-sync-daemon-0.32.0-aarch64-apple-darwin. The install on the Mac whose helper moved (2.2) is pending.
Installed on that Mac 2026-10-30 10:29Z; read back 12:1xZ: both sessions from the helper's account folder are in the hub, under the same projects as the main folder's sessions (the helper's session, 251 messages, current to 12:03Z; and the account's sign-in session).
