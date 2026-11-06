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

- Creating the initial git repository
- Configuring git settings:
  - `init.defaultBranch = main`
  - `pull.rebase = true` (prevents merge commits)
  - `push.default = current`
  - `merge.ff = only` (fast-forward only)
  - `commit.gpgsign = false` (no GPG signing)

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
pub fn sync_remotes(home: &str, machines: &[Machine]) -> Result<(), GitError>
```

## Sync Operations

The `sync.rs` module provides synchronization functions:

### `pull_all()`

Pulls from all configured remotes:

```rust
pub fn pull_all(home: &str, verbose: bool) -> Result<usize, GitError>
```

Returns the number of successful pulls.

### `push_all()`

Pushes to all configured remotes:

```rust
pub fn push_all(home: &str, verbose: bool) -> Result<usize, GitError>
```

Returns the number of successful pushes.

### `sync_git()`

Performs a full sync (pull then push):

```rust
pub fn sync_git(home: &str, verbose: bool)
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
- `<user>`: The user associated with the device
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
sync_git("/home/user", true);

// List configured remotes
let remotes = list_remotes().unwrap();
println!("Remotes: {:?}", remotes);
```

## Verbose Mode

When `verbose` is true, sync operations log their progress:
- "Syncing git..."
- "  . pulled from <remote>"
- "  . pushed to <remote>"
- "Git sync complete."

## Testing

Git operations have integration tests. Note that these tests require a git binary to be available on the system.
