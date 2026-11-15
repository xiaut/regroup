# AGENTS.md for `regroup`

Work through the sections below **in order**; each one is mandatory.

## 1. Fetch Remote Branches First

Before starting ANY work on this repository — and in particular before creating a worktree for a branch or a pull request — fetch all remote branches so that you always start from up-to-date code:

```bash
git fetch --all --prune
```

Do not create branches, worktrees, or PRs from stale local refs. After fetching, base your work on the up-to-date remote-tracking refs (e.g. `origin/<branch>`).

Script S1 — `scripts/bootstrap.sh` (§8) — bundles this fetch with §2.

## 2. Load Project Context

Right after fetching, and before reading any file, planning, or asking questions, run:

```bash
tree -I ".git|target" --dirsfirst
cat Cargo.toml
```

These commands give you the complete project structure and configuration that you need for every task — without them, you lack essential context. For additional documentation, see the `docs/` directory.

Script S1 — `scripts/bootstrap.sh` (§8) — runs §1 + §2 in one step.

## 3. Specification Before Implementation

Unless the task is a well-defined TODO, do not start implementing right away: loop with the developer to understand the need and settle the specification, and wait for an **explicit approval** of that specification before starting to implement.

Working on a new (worktree, branch, PR) is **implicit**: once the specification is approved — or immediately when the task is a well-defined TODO — create the worktree, push the branch, and open the draft PR without asking again.

## 4. Worktrees, Branches, and PRs — Always Inside This Directory

Any work that is not on `main` in the main checkout implies a **worktree** + a dedicated **branch** + a **pull request opened on the first commit** — as a **draft** if the work is not ready yet. Create the worktree **inside this repository directory** — never next to it or anywhere else outside it:

```bash
git worktree add .worktrees/<name> -b <branch> origin/<branch>
```

Working outside the approved working directory triggers an "outside workdir" approval prompt to the developer; a worktree under `.worktrees/` stays inside the repo, so that approval is never asked.

Script S2 — `scripts/new-work.sh <name> <branch> [base-ref]` (§8) — worktree + branch + push in one step.

Rules:

- Use a descriptive worktree name — the branch name or the PR number (e.g. `.worktrees/pr-42`)
- `.worktrees/` is ignored via the committed `.gitignore`, so the main worktree's `git status` stays clean
- Do all work for that branch/PR inside its worktree; do not check out branches in the main worktree
- Push the branch and open the PR on the **first commit** — as a **draft** while the work is not ready — so every branch is backed by an open PR from the start. Script S3 — `scripts/first-commit.sh "<msg>"` (§8) — commit with the required trailer + push + `gh pr create --draft` if missing
- You — the agent — decide when the PR is ready for review: as soon as your work is done, mark the PR **ready for review** yourself (it must no longer be in a draft state) — do not leave it in draft and do not ask the developer to flip it for you. Script S4 — `scripts/finish-pr.sh` (§8) — `gh pr ready` + prints the two test reminders below
- One (worktree, branch, PR) tuple per session: if more work is requested in the same session **and it needs this session's context**, reuse the same worktree and branch — and if that PR has already been merged, open a **new PR on the same branch**, never a second worktree/branch
- **Hand off context-free work to a new session**: a fix or improvement that would need a new PR but does **not** need this session's context — including anything unrelated or only slightly related to the current task — must not be started in this session (the one-tuple rule is enforced, not relaxed). Instead, end your reply with a ready-to-use command that opens a **new session** seeded with an adequate prompt:

  ```bash
  vibe "<prompt>"   # run from the repo root
  ```

  The new session follows §1–§4 on its own and gets its own (worktree, branch, PR) tuple. The prompt must stand alone: state what to change, where, and which AGENTS.md sections apply.
- Keep the worktree in place while the PR is open so the developer can test

When you are done, always end with two copy-paste-ready reminders so the developer can test the PR — fill in the real worktree name and branch name:

```bash
cd .worktrees/<name> && cargo run
```

```bash
git pull && git switch <branch> && cargo run
```

The first one is run from the repo root and tests the PR in its worktree. The second one is for the main checkout and stays the developer's call — never check out the branch there yourself.

## 5. Never Work Outside the Workdir — Temporary Files in `.vibe/tmp/`

Never create, read, modify, or delete anything outside the workdir — not in `/tmp`, not in `$HOME`, not anywhere else. Working outside the approved working directory triggers an "outside workdir" approval prompt to the developer, so keep every file operation inside the worktree.

When temporary or scratch files are needed (prototypes, fetched data, throwaway scripts, working notes), put them in a tmp folder **inside the worktree**, under `.vibe/tmp/`:

```bash
mkdir -p .vibe/tmp
# use .vibe/tmp/<file> instead of /tmp/<file>
```

`.vibe/` is ignored via the committed `.gitignore`, so nothing under it ever shows up in `git status` or gets committed by accident.

## 6. When the PR Is Merged

When the PR is merged — or when the developer explicitly asks — and the branch is no longer needed in this session, clean up in this order:

1. Fetch so the local refs reflect the merge: `git fetch --all --prune`
2. Update the local `main` branch if possible: in the main checkout run `git merge --ff-only origin/main`. Fast-forward only — never reset, never force; if that is not possible (diverged local `main`, conflicting uncommitted changes), leave `main` as is and report it.
3. Remove the worktree: `git worktree remove .worktrees/<name>`, then `git worktree prune`
4. Delete the remote branch — but only after checking that its actual content has been merged into `main`: `git cherry origin/main <branch>` must show no `+` commits (equivalently, `git diff origin/main <branch>` must be empty). Do not rely on `git branch --merged` alone — a squash merge does not make the branch an ancestor of `main`. If anything on the branch is not merged, keep the branch and report what is missing. Only then: `git push origin --delete <branch>`

Do not check out another branch in the main checkout — that stays the developer's call.

Script S5 — `scripts/merged-cleanup.sh <name> <branch>` (§8) — runs steps 1–4 in this order.

## 7. Reporting — Short Citations Only

The developer is considered to be aware of AGENTS.md. When you execute a rule, report it to the user only as a short citation of the rule — section number plus a few words (e.g. "§ 4: worktree under `.worktrees/`") — never as a full re-explanation of the rule.

## 8. Scripts

The recurring processes above are scripted under `scripts/` — run the script instead of re-deriving the commands. All scripts fail fast (`set -euo pipefail`) and stay inside the workdir.

| ID | Script | Covers | Usage |
|---|---|---|---|
| S1 | `scripts/bootstrap.sh` | §1 + §2 | `scripts/bootstrap.sh` (anywhere in the repo) |
| S2 | `scripts/new-work.sh` | §4 worktree + branch + push | `scripts/new-work.sh <worktree-name> <branch> [base-ref=main]` |
| S3 | `scripts/first-commit.sh` | §4 commit + push + draft PR | `scripts/first-commit.sh "<commit message>"` (inside the worktree) |
| S4 | `scripts/finish-pr.sh` | §4 ready-for-review + test reminders | `scripts/finish-pr.sh` (inside `.worktrees/<name>`) |
| S5 | `scripts/merged-cleanup.sh` | §6 post-merge cleanup | `scripts/merged-cleanup.sh <worktree-name> <branch>` (anywhere in the repo) |

S2–S4 automate the PR steps with `gh` (`gh pr create --draft`, `gh pr ready`, `gh pr view`); `gh` must be authenticated on the machine running them. S5 deletes the remote branch only when `git cherry origin/main <branch>` shows no `+` commits.
