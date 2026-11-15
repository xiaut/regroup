# AGENTS.md for `regroup`

## Fetch Remote Branches First (mandatory)

Before starting ANY work on this repository — and in particular before creating a worktree for a branch or a pull request — you must fetch all remote branches so that you always start from up-to-date code:

```bash
git fetch --all --prune
```

**This is mandatory.** Do not create branches, worktrees, or PRs from stale local refs. After fetching, base your work on the up-to-date remote-tracking refs (e.g. `origin/<branch>`), for example:

```bash
git worktree add .worktrees/<name> -b <branch> origin/<branch>
```

## Load Project Context (mandatory)

When you begin ANY work on this repository, right after fetching, you must run:

```bash
tree -I ".git|target" --dirsfirst
cat Cargo.toml
```

**This is mandatory.** Do not read files, do not plan, do not ask questions. Run these commands next, before reading anything else.

### Why

These commands provide the complete project structure and configuration that you need for every task. Without them, you lack essential context.

For additional documentation, see the `docs/` directory.

## Worktrees, Branches, and PRs — Always Inside This Directory

Any work that is not on `main` in the main checkout implies a **worktree** + a dedicated **branch** + a **pull request opened on the first commit** — as a **draft** if the work is not ready yet. The worktree must be created **inside this repository directory** — never next to it or anywhere else outside it:

```bash
git worktree add .worktrees/<name> <branch>
```

**This is mandatory.** Working outside the approved working directory triggers an "outside workdir" approval prompt to the developer. A worktree under `.worktrees/` stays inside the repo, so that approval is never asked.

Rules:

- Use a descriptive worktree name — the branch name or the PR number (e.g. `.worktrees/pr-42`)
- `.worktrees/` is ignored via the committed `.gitignore`, so the main worktree's `git status` stays clean
- Do all work for that branch/PR inside its worktree; do not check out branches in the main worktree
- Push the branch and open the PR on the **first commit** — as a **draft** while the work is not ready — so every branch is backed by an open PR from the start
- Do not remove the worktree when the work is done — keep it in place so the developer can test while the PR is open; clean up with `git worktree remove .worktrees/<name>` followed by `git worktree prune` only after the PR is merged or when the developer explicitly asks
