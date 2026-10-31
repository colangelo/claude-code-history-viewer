# Residuals

Known, accepted gaps with an end condition. Remove an entry when its condition is met.

- **Distiller routing: no working folder from daemons without `ClaudeMessage.cwd`** (`distill-route-full-session`, re-test C8). Until both Macs run the daemon that stores `cwd`, their new messages carry none, and the routing view has no working-folder signal for them; the path, tool-input and text signals still apply. Ends: the read-back after both daemon installs shows `cwd` on new messages from each Mac.
