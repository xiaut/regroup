#!/usr/bin/env bash
# S4 — AGENTS.md §4: mark the PR ready for review (the agent decides, do not
# leave it in draft) and print the two copy-paste test reminders — each one
# only if accurate: the worktree exists for the first, the branch exists for
# the second. Whether `cargo run` actually exercises the change is the agent's
# judgment in its reply (§4).
# Usage: scripts/finish-pr.sh   (run inside the .worktrees/<name> worktree)
set -euo pipefail
branch=$(git branch --show-current)
gh pr ready "$branch"
top=$(git rev-parse --show-toplevel)
case "$top" in
  */.worktrees/*) name=${top##*/.worktrees/} ;;
  *) echo "error: run this from inside a .worktrees/<name> worktree" >&2; exit 1 ;;
esac
if git worktree list --porcelain | grep -q "^worktree .*/\.worktrees/$name$"; then
  echo
  echo "cd .worktrees/$name && cargo run"
fi
if git show-ref --verify --quiet "refs/heads/$branch" || git show-ref --verify --quiet "refs/remotes/origin/$branch"; then
  echo
  echo "git pull && git switch $branch && cargo run"
fi
