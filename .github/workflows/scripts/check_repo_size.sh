#!/usr/bin/env bash
#
# Guardrail: the repository stays small enough to clone on a studio wifi.
#
# WHY
#   A per-file limit misses the other failure mode: a hundred files just under
#   it, or a large file committed once and then deleted, which stays in history
#   forever. History size is the number that actually costs a contributor time,
#   and once something big is in it, removing it means a force-push.
#
#   Wit's own README argues that version history should be cheap. A bloated repo
#   would be an unforced own goal.
#
# USAGE
#   .github/workflows/scripts/check_repo_size.sh [max_history_kb] [max_worktree_kb]
#
# NOTE
#   Run this on a full clone (actions/checkout with fetch-depth: 0). On a shallow
#   clone the history number is meaningless, and the script says so rather than
#   passing quietly.

set -euo pipefail

MAX_HISTORY_KB="${1:-25600}"    # 25 MiB of .git
MAX_WORKTREE_KB="${2:-10240}"   # 10 MiB of tracked files

fail=0

if [ "$(git rev-parse --is-shallow-repository 2>/dev/null || echo true)" = "true" ]; then
  echo "::warning::Shallow clone — history size not checked. Use fetch-depth: 0 for a real measurement."
  history_kb=0
else
  git count-objects -v || true
  history_kb=$(du -sk .git | cut -f1)
fi

# Batched, so a large file list cannot silently truncate the sum. The awk filter
# drops xargs' per-batch "total" lines and keeps the per-file ones.
worktree_kb=$(git ls-files -z \
  | xargs -0 -n 200 wc -c \
  | awk '$2 != "total" { t += $1 } END { printf "%d", t / 1024 + 1 }')

printf '  tracked files : %s KB (limit %s KB)\n' "$worktree_kb" "$MAX_WORKTREE_KB"
printf '  git history   : %s KB (limit %s KB)\n' "$history_kb" "$MAX_HISTORY_KB"

if [ "$worktree_kb" -gt "$MAX_WORKTREE_KB" ]; then
  echo "::error::Tracked files total ${worktree_kb} KB, over the ${MAX_WORKTREE_KB} KB ceiling."
  fail=1
fi

if [ "$history_kb" -gt "$MAX_HISTORY_KB" ]; then
  echo "::error::.git is ${history_kb} KB, over the ${MAX_HISTORY_KB} KB ceiling."
  cat <<'EOF'

  Something large is in the history. Note that deleting the file in a later
  commit does not shrink this — the object is still there. Find it with:

      git rev-list --objects --all \
        | git cat-file --batch-check='%(objecttype) %(objectname) %(objectsize) %(rest)' \
        | awk '$1=="blob"' | sort -k3 -n -r | head -20

  If it is audio or a DAW project it should never have been committed; that
  needs a history rewrite, not a follow-up commit. Ask before force-pushing.
EOF
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  echo "clean: repository size within limits"
fi
exit "$fail"
