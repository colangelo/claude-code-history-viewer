#!/usr/bin/env bash
# Refuse a commit whose staged copy of a file differs from the file on disk.
#
# WHY: this repo's .git is Syncthing-shared between the Macs, so .git/index can
# arrive from the other Mac describing an OLDER version of a file than the one
# in the working tree. `git status` then shows `MM <file>`, and a commit takes
# the stale index blob, silently reverting whatever the working tree (and HEAD)
# had. On 2026-09-28 the index held a16bcbe7 for crates/history-core/src/utils.rs
# while HEAD and the worktree held the e7c8e5f4 fix; the next commit would have
# undone the fix with nothing looking wrong.
#
# The rule: every staged file's index blob must equal `git hash-object` of the
# working-tree file (same filters `git add` applies). Deliberate partial
# staging (`git add -p`) is the one legitimate exception:
#   CCHV_ALLOW_PARTIAL_STAGE=1 git commit ...
set -euo pipefail

[ "${CCHV_ALLOW_PARTIAL_STAGE:-}" = 1 ] && exit 0

bad=()
while IFS= read -r -d '' f; do
  [ -f "$f" ] || continue                       # deleted/renamed-away: nothing on disk to compare
  staged=$(git ls-files -s -- "$f" | awk 'NR==1 {print $2}')
  [ -n "$staged" ] || continue
  [ "$(git ls-files -s -- "$f" | awk 'NR==1 {print $1}')" = 160000 ] && continue   # submodule
  ondisk=$(git hash-object -- "$f")
  [ "$staged" = "$ondisk" ] || bad+=("$f  (index ${staged:0:8}, disk ${ondisk:0:8})")
done < <(git diff --cached --name-only --diff-filter=ACMRT -z)

if [ "${#bad[@]}" -gt 0 ]; then
  {
    echo "pre-commit: the staged copy differs from the file on disk:"
    printf '  %s\n' "${bad[@]}"
    echo
    echo "If you did NOT stage a partial change on purpose, the index is probably a stale"
    echo "copy that arrived through the Syncthing-shared .git. Committing now would record"
    echo "the old content. Fix: 'git add <file>' (or 'git reset -q -- <file>' to drop it"
    echo "from the commit), check 'git diff --cached', and commit again."
    echo "Deliberate partial staging: CCHV_ALLOW_PARTIAL_STAGE=1 git commit ..."
  } >&2
  exit 1
fi
