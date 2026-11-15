#!/usr/bin/env bash
# S4 — AGENTS.md §4: mark the PR ready for review (the agent decides, do not
# leave it in draft) and print the two copy-paste test reminders.
# Usage: scripts/finish-pr.sh   (run inside the .worktrees/<name> worktree)
set -euo pipefail
branch=$(git branch --show-current)
gh pr ready "$branch"
top=$(git rev-parse --show-toplevel)
case "$top" in
  */.worktrees/*) name=${top##*/.worktrees/} ;;
  *) echo "error: run this from inside a .worktrees/<name> worktree" >&2; exit 1 ;;
esac
echo
echo "cd .worktrees/$name && cargo run"
echo
echo "git pull && git switch $branch && cargo run"
