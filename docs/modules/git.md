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

### `pull_all()`

Pulls from all configured remotes:

```rust
pub fn pull_all(home: &str) -> Result<usize, GitError>
```

Returns the number of successful pulls.

### `push_all()`

Pushes to all configured remotes:

```rust
pub fn push_all(home: &str) -> Result<usize, GitError>
```

Returns the number of successful pushes.

### `sync_git()`

Performs a full sync (pull then push):

```rust
pub fn sync_git(home: &str, temp_users: &HashMap<String, String>)
```

This is the main function called when the user presses 's' in interactive mode.

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
use crate::git::{sync_git, list_remotes};

// Sync with all peers
sync_git("/home/user", &temp_users);

// List configured remotes
let remotes = list_remotes().unwrap();
println!("Remotes: {:?}", remotes);
```

## Logging

There is no verbose mode: sync operations always log their progress (see the
[Logging policy](../logging.md)):
- "Syncing git..."
- A line before each pull/push explaining that a password prompt may appear
- "  - pulled from <remote>" / "  - pushed to <remote>" for every remote
- "Git sync complete."

## Testing

Git operations have integration tests. Note that these tests require a git binary to be available on the system.
