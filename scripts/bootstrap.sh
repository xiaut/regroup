#!/usr/bin/env bash
# T1 — AGENTS.md §1 + §2: fetch all remote branches, then load project context.
# Usage: scripts/bootstrap.sh   (run from anywhere inside the repo)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git fetch --all --prune
tree -I ".git|target" --dirsfirst
cat Cargo.toml
