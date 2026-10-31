# Residuals

Known, accepted gaps with an end condition. Remove an entry when its condition is met.

- **Distiller routing: no working folder from daemons without `ClaudeMessage.cwd`** (`distill-route-full-session`, re-test C8). Until both Macs run the daemon that stores `cwd`, their new messages carry none, and the routing view has no working-folder signal for them; the path, tool-input and text signals still apply. Ends: the read-back after both daemon installs shows `cwd` on new messages from each Mac.
- **Journal reads: a paraphrase handed to another project is not caught** (`journal-read-protection-tag`, re-test F3 / C7 step 2; accepted by ac 2026-10-31 on the cchv-v0.34.0 release row, option (a)). A session that reads a protected journal entry and writes a paraphrase into another project's group (peer message, issue comment, commit, file) leaves no tag there, so that group can route general. Default reads still return non-general rows (tagged). Ends: Gitea #95 (default-exclude non-general rows after labelling), due 2026-11-14.
