# git Module

The `git` module handles git repository management and synchronization.

## Overview

This module manages the configuration git repository at `~/.regroup-config/git/`. It provides functions to initialize the repository, add/remove remotes, and sync with peer devices.

## Module Structure

```
git/
├── mod.rs         # Module exports
├── repo.rs        # Git repository initialization and checks
├── remote.rs      # Remote management (add/remove/list)
└── sync.rs        # Pull/push operations, commit helpers
```

## Repository Initialization

The `repo.rs` module handles:

- Creating the initial git repository with `git init -b main` (falling back
  to `symbolic-ref HEAD refs/heads/main` on git < 2.28)
- Configuring git settings (repo-local). The set is deliberately **minimal**: the
  `tmp/` overlay keeps unconfirmed data out of the repository and `settings.json`
  is local-only (outside the repo), so `git/` contains only self-authored files and
  no extra conflict-avoidance settings are needed. What remains is only what
  unattended, decentralized sync requires:
  - `user.name` / `user.email` — commits are created automatically
  - `commit.gpgsign = false` — never block on a signing prompt
  - `core.editor = true` / `sequence.editor = true` — never open an interactive editor
  - `receive.denyCurrentBranch = updateInstead` — peers push to this checked-out branch
  - `pull.rebase = true` — the first sync between two independently bootstrapped
    repos has unrelated histories; rebase is what makes that pull succeed at all,
    and it gives a linear history
  - `fetch.prune = true` — drop remote-tracking refs of removed devices
- Redundant settings are intentionally **not** set: `merge.ff` (shadowed by
  `pull.rebase = true`) and `push.default` (every push passes an explicit
  `<remote> main` refspec)

## Repository Checks

`check_repo()` verifies:
- The repository exists
- Required git settings are configured
- The working directory is clean

## Remote Management

The `remote.rs` module provides:

### `add_remote()`

Adds a git remote for a device:

```rust
pub fn add_remote(name: &str, url: &str) -> Result<(), GitError>
```

Remote names use the device name from the configuration.

### `remove_remote()`

Removes a git remote:

```rust
pub fn remove_remote(name: &str) -> Result<(), GitError>
```

### `list_remotes()`

Lists all configured remotes:

```rust
pub fn list_remotes() -> Result<Vec<String>, GitError>
```

### `sync_remotes()`

Updates all remotes from the current configuration:

```rust
pub fn sync_remotes(home: &str, devices: &[Device]) -> Result<(), GitError>
```

## Sync Operations

The `sync.rs` module provides synchronization functions:

### `git_pull_remote()` / `git_push_remote()`

Pull from / push to one remote. The target may be a remote name **or a
URL** — git accepts a URL wherever a remote name is expected — which is how
per-user sync targets (no named remote) are reached:

```rust
pub fn git_pull_remote(repo_path: &Path, remote_name: &str) -> io::Result<()>
pub fn git_push_remote(repo_path: &Path, remote_name: &str) -> io::Result<()>
```

### `sync_with_targets()`

Performs a scoped sync (pull from all targets, then push to all targets):

```rust
pub struct SyncTarget { pub label: String, pub target: String }
pub fn sync_with_targets(repo_path: &Path, targets: &[SyncTarget]) -> Vec<String>
```

Each target is one `user@device` pair: `target` is the git URL
(`user@device:~/.regroup-config/git`, see `remote::remote_url`), `label` is
how it appears in the logs. This is what `ops::sync::sync_git()` calls when
the user presses 's' in interactive mode, with the targets derived from the
sync scope: every device × every user on the main menu, one device × every
user on a selected device row, and every device × every user except the
selected device pinned to the selected user in the Users pane.

### `commit_helper()`

Helper function to create a commit:

```rust
pub fn commit_helper(home: &str, message: &str) -> Result<String, GitError>
```

### `git_is_tracked()`

Checks if a file is tracked by git:

```rust
pub fn git_is_tracked(path: &Path) -> bool
```

## Git URL Format

Remotes use the following URL format:

```
ssh://<user>@<ip>:<port>/.regroup-config/git
```

Where:
- `<user>`: The device's SSH user, resolved through the config proxy — a
  temporary user (persisted at `tmp/devices/<name>/users/<user>.json`, purged
  once the device declares the same user in `git/`) or a user the device
  declared in `users/`; never announced on the network
- `<ip>`: The device's IP address
- `<port>`: The SSH port (default: 22)

Note: SSH key-based authentication is used (no password prompts).

## Error Handling

Git-related errors:

- `GitError::RepoNotFound`: Repository doesn't exist
- `GitError::NotAGitRepo`: Path is not a git repository
- `GitError::GitCommandFailed`: Git command returned an error
- `GitError::RemoteNotFound`: Remote doesn't exist
- `GitError::IoError`: I/O error

## Usage Example

```rust
use crate::git::{sync_git, SyncScope};
use std::collections::HashMap;

// Sync with all peers, every user
sync_git("/home/user", &HashMap::new(), &SyncScope::All);

// Sync one device, every user of it
sync_git("/home/user", &HashMap::new(), &SyncScope::Device("laptop".into()));
```

## Logging

There is no verbose mode: sync operations always log their progress (see the
[Logging policy](../logging.md)):
- "Syncing git (<scope>)..." — the scope (every device/every user, one
  device, or a pinned user@device) is stated up front
- A line before each pull/push explaining that a password prompt may appear
- "  - pulled from <user@device>" / "  - pushed to <user@device>" for every target
- "Git sync complete."

## Testing

Git operations have integration tests. Note that these tests require a git binary to be available on the system.
