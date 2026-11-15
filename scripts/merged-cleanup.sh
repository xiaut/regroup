#!/usr/bin/env bash
# S5 — AGENTS.md §6: post-merge cleanup, in the mandated order.
# Usage: scripts/merged-cleanup.sh <worktree-name> <branch>   (run from anywhere in the repo)
set -euo pipefail
name=${1:?usage: merged-cleanup.sh <worktree-name> <branch>}
branch=${2:?missing <branch>}
root=$(cd "$(git rev-parse --git-common-dir)/.." && pwd)
main_wt=$(git -C "$root" worktree list --porcelain | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt}')

# 1. Fetch so the local refs reflect the merge.
git -C "$root" fetch --all --prune
# 2. Fast-forward the main checkout only; never reset, never force.
if git -C "$main_wt" merge --ff-only origin/main; then
  echo "main fast-forwarded to origin/main"
else
  echo "main is not fast-forwardable; left as is (§6)" >&2
fi
# 3. Remove the worktree.
git -C "$root" worktree remove "$root/.worktrees/$name"
git -C "$root" worktree prune
# 4. Delete the remote branch only if its content is fully in origin/main.
#    git cherry can show '+' after a squash merge, so gate on an empty diff instead (§6).
if [ -n "$(git -C "$root" diff origin/main "$branch")" ]; then
  echo "branch $branch still differs from origin/main; keeping it (§6)" >&2
  exit 1
fi
git -C "$root" push origin --delete "$branch"
