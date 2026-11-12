# interactive Module

The `interactive` module contains the main interactive loop and state management.

## Overview

This module is the heart of regroup's interactive mode. It listens for verified beacons, handles user input, and manages the pending device queue, signing key rotation prompts, and rename prompts.

All decisions that affect what is written to disk live in this module.

## Module Structure

```
interactive/
├── mod.rs          # Module root with public re-exports
├── state.rs        # InteractiveState + all implementation
└── utils.rs        # Utility functions
```

## Public Types

### `Context`

Per-run context shared by every branch of the interactive loop:

```rust
pub struct Context<'a> {
    pub home: &'a str,                  // User's home directory
    pub ssh_config: &'a Path,           // Path to ~/.ssh/config
    pub dry_run: bool,                  // If true, no modifications
    pub local_public_key: &'a SshPublicKey, // This device's SSH key
    pub verbose: bool,                  // Verbose mode
    pub local_name: &'a str,            // This device's hostname
    pub local_ip: Option<&'a str>,       // This device's IP
}
```

## Main Loop

The interactive loop is implemented in `run()`:

```rust
pub fn run(ctx: &Context, udp_port: u16, beacons: &Receiver<BeaconDevice>) -> usize
```

It returns the number of devices accepted during the run.

### Loop Flow

1. Check for new beacons from the listener thread
   - If a beacon is fully handled, skip to next iteration
   
2. Poll for user actions
   - If user requests quit, exit loop
   
3. Update UI display
   - Set current question based on state
   - Update detected and added device panels
   - Set selection highlights
   - Draw the UI

### Beacon Handling

Each received beacon goes through `InteractiveState::handle_beacon()`:

1. **Self-filtering**: Ignore beacons from this device (using SSH public key)
2. **Pulse**: Animate the device's row (user sees the app receiving)
3. **Signing key pinning**: For known devices, verify the beacon's signing key matches the pinned key
   - If different: prompt user to trust new key (reinstall) or reject (spoofing)
4. **Known device**: Check if IP changed
   - If changed: verify host key at new IP
   - If verified: update SSH config and known_hosts
   - If mismatch: SECURITY - reject and warn
5. **New device**: Add to pending queue and prompt user

## State Management

### `InteractiveState`

All mutable state of the interactive loop:

```rust
pub struct InteractiveState {
    // Known devices
    pub known_keys: HashSet<SshPublicKey>,
    pub key_to_machine: HashMap<SshPublicKey, Device>,
    pub pinned_signing_keys: HashMap<SshPublicKey, SigningPublicKeyB64>,
    
    // Signing key rotation
    pub rejected_signing_keys: HashSet<(SshPublicKey, SigningPublicKeyB64)>,
    pub rotation_pending: Option<BeaconDevice>,
    
    // Pending devices
    pub pending_devices: VecDeque<BeaconDevice>,
    pub pending_keys: HashSet<SshPublicKey>,
    pub pending_selected: Option<SshPublicKey>,
    
    // Known IPs
    pub known_ip: HashMap<SshPublicKey, String>,
    pub rejected_keys: HashSet<SshPublicKey>,
    
    // Rename
    pub rename_state: Option<(BeaconDevice, String)>,
    
    // Delete
    pub delete_selection: Option<String>,
    
    // Wipe
    pub wipe_pending: bool,
    
    // Setup-cd
    pub setup_cd_pending: bool,
    
    // SSH key management
    pub temp_users: HashMap<String, String>,
    pub users_selected: Option<String>,
    pub temp_user_pending: Option<String>,
    
    // Settings
    pub verbose: bool,
    pub new_count: usize,
}
```

### State Initialization

`InteractiveState::new(ctx: &Context)`:
- Creates empty collections
- Calls `reload()` to load existing configuration from disk
- Initializes verbose mode from context

### Reloading

`InteractiveState::reload(ctx: &Context)`:
- Rebuilds lookup maps from device files on disk
- Useful after git sync pulls new device files
- Cleans up local traces for devices removed by peers

## Views

### `current_question()`

Determines what question to display based on the current state:

- Wipe pending → Wipe confirmation
- Setup-cd pending → Setup-cd confirmation
- Delete selection → Delete confirmation
- Rename state → Rename prompt
- Rotation pending → Trust key prompt
- Install/remove key mode → Key management questions
- Plain mode with pending devices → Add device prompt
- Default → Idle

### `detected_view()`

Returns devices to show in the Detected panel:
- Pending devices (announced but not accepted)
- Sorted by name
- With SSH public key as identifier

### `added_view()`

Returns devices to show in the Added panel:
- Known devices from configuration
- With last known IP from state
- Sorted by name

## Action Handling

`InteractiveState::handle_action()` processes user actions:

### Device Actions

| Action | Behavior |
|--------|----------|
| `Quit` | Return true to exit loop |
| `Sync` | Run `sync_git()` and reload state |
| `Add` | Accept selected device from Detected panel |
| `Ignore` | Reject selected device from Detected panel |
| `AddCurrent` | Accept front of pending queue (plain mode) |
| `SkipCurrent` | Reject front of pending queue (plain mode) |

### Trust Actions

| Action | Behavior |
|--------|----------|
| `TrustKey` | Trust the pending signing key rotation |
| `RejectKey` | Reject the pending signing key rotation |

### Rename Actions

| Action | Behavior |
|--------|----------|
| `Rename(name)` | Rename device to `name` |
| `SkipRename` | Cancel rename prompt |

### Delete Actions

| Action | Behavior |
|--------|----------|
| `DeleteMode` | Enter delete mode |
| `SelectPrev/Next` | Navigate delete selection |
| `DeleteConfirm` | Confirm deletion of selected device |
| `DeleteCancel` | Exit delete mode |

### Wipe Actions

| Action | Behavior |
|--------|----------|
| `WipeMode` | Show wipe confirmation |
| `WipeConfirm` | Wipe entire configuration |
| `WipeCancel` | Cancel wipe |

### Setup-cd Actions

| Action | Behavior |
|--------|----------|
| `SetupCdMode` | Show setup-cd confirmation |
| `SetupCdConfirm` | Install regroup-cd shell function |
| `SetupCdCancel` | Cancel setup-cd |

### Verbose Actions

| Action | Behavior |
|--------|----------|
| `ToggleVerbose` | Toggle verbose mode on/off |

### Help Actions

| Action | Behavior |
|--------|----------|
| `Help` | Show CLI help text |

## Device Management

### Adding Devices

1. User selects device from Detected panel or accepts front of queue
2. Check if name is already taken
   - If yes: prompt for new name (rename_state)
   - If no: proceed to add
3. Add device to configuration via `add_device_to_config()`
4. Pin the signing key
5. Add to known_keys and key_to_machine maps
6. Record IP in known_ip

### Ignoring Devices

- Device is removed from pending queue
- SSH public key added to rejected_keys
- Device will be ignored for the rest of the run

### Deleting Devices

1. User selects device from Added panel (machine action), or machine+user
   actions go through the Users sub-panel (Enter on the machine) with a user
   selected
2. Confirm deletion
3. Remove device file from configuration
4. Remove SSH config Host block
5. Remove git remote
6. Remove from in-memory maps

### Renaming Devices

1. Device name collision detected
2. Generate unique suggestion with `unique_device_name()`
3. Prompt user for new name
4. If name is also taken, generate new suggestion
5. Add device with new name

## Wipe Configuration

`wipe_config()`:
- Removes entire `.regroup-config` directory
- Removes all regroup-managed Host blocks from `~/.ssh/config`
- Cleans up known_hosts entries for all known IPs
- Drops all in-memory state
- Recreates empty configuration skeleton

## Git Sync

`sync_git()`:
- Pulls from all configured remotes
- Pushes to all configured remotes
- Logs progress if verbose mode is on
- Reloads state after pull to include new devices

## Dry Run Mode

When `ctx.dry_run` is true:
- No files are modified
- No git operations performed
- Actions are logged but not executed
- Useful for testing without side effects

## Utility Functions

### `key_preview()`

Creates a short preview of an SSH public key for display:

```rust
pub fn key_preview(key: &SshPublicKey) -> String
```

Example: `"ssh-ed25519 AA..."`
