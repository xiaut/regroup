#!/usr/bin/env bash
# T2 — AGENTS.md §4: create the worktree inside .worktrees/, create/reuse the
# branch, and push it so it is backed by a remote branch from the start.
# The draft PR is opened on the first commit — see scripts/first-commit.sh.
# Usage: scripts/new-work.sh <worktree-name> <branch> [base-ref=main]
set -euo pipefail
name=${1:?usage: new-work.sh <worktree-name> <branch> [base-ref=main]}
branch=${2:?missing <branch>}
base=${3:-main}
root=$(cd "$(git rev-parse --git-common-dir)/.." && pwd)
cd "$root"
git fetch --all --prune
if git ls-remote --exit-code --heads origin "$branch" >/dev/null; then
  # §4: one (worktree, branch, PR) per session — reuse the existing remote branch.
  git worktree add ".worktrees/$name" -b "$branch" "origin/$branch"
else
  git worktree add ".worktrees/$name" -b "$branch" "origin/$base"
fi
git -C ".worktrees/$name" push -u origin "$branch"
