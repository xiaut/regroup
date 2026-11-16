# interactive Module

The `interactive` module contains the main interactive loop and state management.

> **Target of the upcoming refactor:** `interactive` is a front-end of the
> **binary** crate (`src/main.rs`); it calls the features in the library's
> `regroup::ops` layer (device add/remove/wipe, sync, setup-cd, help text)
> instead of owning them in a `cli` module.

## Overview

This module is the heart of regroup's interactive mode. It listens for verified beacons, handles user input, and manages the pending device queue, the new-signing-key super-state, and rename prompts.

All decisions that affect what is written to disk live in this module; the writes themselves are performed by `ops`.

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
    pub dry_run: bool,                  // If true, no modifications (front-end capability, no binary flag)
    pub local_public_key: &'a SshPublicKey, // This device's SSH key
    pub local_name: &'a str,            // This device's hostname
    pub local_ip: Option<&'a str>,       // This device's IP
}
```

The UI backend (full-screen vs plain) is auto-detected from stdout in the
target interface; there is no `--cli` flag anymore.

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
   - If different: mark the row under the `── new signing key ──` super-state of the Devices pane (reinstall or spoofing) — passively, never as a blocking question; the user trusts (`t`) or rejects (`n`) from the row, and the beacons stay ignored until then
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
    pub key_to_device: HashMap<SshPublicKey, Device>,
    pub pinned_signing_keys: HashMap<SshPublicKey, SigningPublicKeyB64>,
    
    // Signing key rotation: added devices presenting a new signing key,
    // shown passively under the `── new signing key ──` super-state group
    pub rejected_signing_keys: HashSet<(SshPublicKey, SigningPublicKeyB64)>,
    pub new_signing_keys: HashMap<SshPublicKey, SigningPublicKeyB64>,
    
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
    // (device, user) waiting for the "install key?" answer that closes
    // the add-user flow (remote receive-add-local XOR receive-install-key,
    // then a device-scoped sync)
    pub install_key_pending: Option<(String, String)>,
    
    // Settings
    pub new_count: usize,
}
```

### State Initialization

`InteractiveState::new(ctx: &Context)`:
- Creates empty collections
- Calls `reload()` to load existing configuration from disk

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

### `devices_view(ctx)`

Returns the rows of the Devices panel, in display order:
- The local device first (name, IP, SSH port from the context)
- Then the added devices: known devices from configuration, with their
  last known IP from state, sorted by name
- Then the detected devices: pending devices (announced but not
  accepted), sorted by name, with SSH public key as identifier

Each row carries its section (`Local` / `Added` / `Detected`); the pane
renders a divider row between sections.

## Action Handling

`InteractiveState::handle_action()` processes user actions:

### Device Actions

| Action | Behavior |
|--------|----------|
| `Quit` | Return true to exit loop |
| `Sync` | Run `ops::sync::sync_git()` with the scope derived from the current pane and selections (main menu: `All`; Devices pane, added row: `Device`; Users pane: `User`), then reload state |
| `Add` | Accept the detected device selected in the Devices panel |
| `Ignore` | Reject the detected device selected in the Devices panel |
| `DevicesSelectPrev/Next` | Move the selection in the Devices panel (local → added → detected rows) |
| `AddCurrent` | Accept front of pending queue (plain mode) |
| `SkipCurrent` | Reject front of pending queue (plain mode) |

### Trust Actions

Row actions on an added device of the `── new signing key ──` group (keys
`t`/`n`) — never a blocking question:

| Action | Behavior |
|--------|----------|
| `TrustKey` | Trust the new signing key presented by the selected added row: pin it and pull the device's authoritative `device.json` |
| `RejectKey` | Reject the new signing key presented by the selected added row: its beacons stay ignored |

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
| `WipeConfirm` | Wipe entire configuration, then quit |
| `WipeCancel` | Cancel wipe |

### Setup-cd Actions

| Action | Behavior |
|--------|----------|
| `SetupCdMode` | Show setup-cd confirmation |
| `SetupCdConfirm` | Install regroup-cd shell function |
| `SetupCdCancel` | Cancel setup-cd |

### Help Actions

| Action | Behavior |
|--------|----------|
| `Help` | Show the help text (`ops::help::usage_text`) |

## Device Management

### Adding Devices

1. User selects a detected row in the Devices panel (key `a`) or accepts
   the front of the queue (plain mode)
2. Check if name is already taken
   - If yes: prompt for new name (rename_state)
   - If no: proceed to add
3. Add device to configuration via `ops::devices::add_device_to_config()`
4. Pin the signing key
5. Add to known_keys and key_to_device maps — the device's row moves from
   the detected section to the added section of the Devices panel
6. Record IP in known_ip

### Ignoring Devices

- Device is removed from pending queue
- SSH public key added to rejected_keys
- Device will be ignored for the rest of the run

### Deleting Devices

1. User selects an added row in the Devices panel (device action, key `d`),
   or device+user actions go through the Users sub-panel (directional
   `Right`/`Enter` from the focused Devices panel with an added row
   selected) with a user selected
2. Confirm deletion (plain mode)
3. Remove the device's files through the proxy (both the `git/` and `tmp/` layers)
4. Remove SSH config Host block
5. Remove git remote
6. Refresh the in-memory maps from the merged proxy view

### Renaming Devices

1. Device name collision detected
2. Generate unique suggestion with `ops::devices::unique_device_name()`
3. Prompt user for new name
4. If name is also taken, generate new suggestion
5. Add device with new name

## Wipe Configuration

`ops::devices::wipe()` (returns `Result<(), String>`):
- Removes entire `.regroup-config` directory
- Removes all regroup-managed Host blocks from `~/.ssh/config`
- Cleans up the known_hosts entries of the last known IPs (recorded in
  memory only, cleaned before quitting)
- Quits the program: the in-memory state derived from the wiped
  configuration dies with the process, so it needs no cleanup; the next
  run recreates the empty configuration skeleton via `load_config()`

## Git Sync

`ops::sync::sync_git(home, temp_users, scope)` — the scope comes from
`InteractiveState::sync_scope()`:

- Main menu (and plain mode): `SyncScope::All` — every device, every user
- Devices pane, added row selected: `SyncScope::Device(name)` — every user
  of that device
- Users pane of an added device: `SyncScope::User { device, user }` —
  every device and user, except the selected device, synced as the
  selected user only. Not offered on the local device's Users pane: the
  local device has no remote, so sync is unavailable on the current user

Each in-scope `user@device` pair is pulled from and pushed to through its
own git URL, so each may prompt for its own password. The named remotes
(one per device, primary user) are still configured for housekeeping. The
result of each pull/push is logged (see [Logging policy](../logging.md)),
and the state is reloaded after the pull to include new devices.

## Dry Run Mode

When `ctx.dry_run` is true:
- No files are modified
- No git operations performed
- Actions are logged but not executed
- Useful for testing without side effects

In the target interface there is no `regroup test` flag: dry run is a
`Context` capability that front-ends and tests can enable, not a binary
subcommand.

## Utility Functions

### `key_preview()`

Creates a short preview of an SSH public key for display:

```rust
pub fn key_preview(key: &SshPublicKey) -> String
```

Example: `"ssh-ed25519 AA..."`
