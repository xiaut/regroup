# AGENTS.md for `regroup`

## Load Project Context First (mandatory)

When you begin ANY work on this repository, your first action must be to run:

```bash
tree -I ".git|target" --dirsfirst
cat Cargo.toml
```

**This is mandatory.** Do not read files, do not plan, do not ask questions. Run these commands first.

### Why

These commands provide the complete project structure and configuration that you need for every task. Without them, you lack essential context.

For additional documentation, see the `docs/` directory.

## Worktrees, Branches, and PRs — Always Inside This Directory

When the developer asks you to work in a **worktree**, on a **branch**, or on a **pull request**, you must create the git worktree **inside this repository directory** — never next to it or anywhere else outside it:

```bash
git worktree add .worktrees/<name> <branch>
```

**This is mandatory.** Working outside the approved working directory triggers an "outside workdir" approval prompt to the developer. A worktree under `.worktrees/` stays inside the repo, so that approval is never asked.

Rules:

- Use a descriptive worktree name — the branch name or the PR number (e.g. `.worktrees/pr-42`)
- `.worktrees/` is ignored via the committed `.gitignore`, so the main worktree's `git status` stays clean
- Do all work for that branch/PR inside its worktree; do not check out branches in the main worktree
- When done, clean up with `git worktree remove .worktrees/<name>` followed by `git worktree prune`
