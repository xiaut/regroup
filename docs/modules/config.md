# config Module

The `config` module handles configuration loading, validation, and storage.

## Overview

This module manages the device configuration stored under `~/.regroup-config/`. All access goes through a **proxy** that merges two layers:

- `~/.regroup-config/git/` — the git repository, source of truth, synced with peers
- `~/.regroup-config/tmp/` — temporary overlay for not-yet-authoritative data (beacon-derived placeholders, temporary users); never committed or synced

Reads merge the layers as `git/ ∪ (tmp/ − git/)`: on overlap `git/` wins and the stale `tmp/` copy is purged (lazily on read, eagerly after each sync). Deletes apply to both layers. All state therefore lives on disk in `git/` + `tmp/`; in-memory structures are only a cache of this merged view. See [Storage Rules](../../storage.md#configuration-proxy-the-tmp-overlay).

## Module Structure

```
config/
├── mod.rs          # Device/Conf types, storage constants, path helpers
├── loading.rs      # load_config(): ensure repo, scan devices/, auto-create local device
├── device.rs       # device file (device.json) load/save + validation tests
├── connection.rs   # connection files: naming rule, ownership enforcement, loader
├── users.rs        # users/<user>.json: one file per known user
├── settings.rs     # settings.json (udp_port, wait_secs)
├── validation.rs   # Device sanity checks (localhost, empty keys, ...)
└── json_helpers.rs # Internal JSON parsing helpers
```

## Key Types

### Device

Represents a device in the configuration:

```rust
pub struct Device {
    pub name: String,
    pub ssh_public_key: SshPublicKey,
    pub signing_public_key: SigningPublicKeyB64,
    pub port: u16,
    pub port_explicit: bool,  // True if port was explicitly set
}
```

### Conf

The root configuration loaded from the repository:

```rust
pub struct Conf {
    pub devices: Vec<Device>,
    pub settings: Settings,
}
```

## Storage Constants

- `CONFIG_DIR`: `".regroup-config"`
- `GIT_DIR`: `"git"` — source-of-truth repository: `$HOME/.regroup-config/git/`
- `TMP_DIR`: `"tmp"` — temporary overlay: `$HOME/.regroup-config/tmp/` (never synced)

## Loading Process

`load_config()` performs the following:

1. Ensures the repository exists (creates it if not)
2. Scans `devices/` **through the proxy**: merges `git/devices/` with `tmp/devices/`, where `git/` wins on overlap and overlapping `tmp/` files are purged
3. Auto-creates a local device entry if not present
4. Records the current user in the local device's own `users/` (in `git/`);
   a temporary user of another device is recorded under `tmp/` instead
5. Loads settings from `settings.json` (in `git/`)
6. Purges `tmp/` entries that are now present in `git/` (lazy purge) (in `git/`)

## Validation

The `validation` module enforces the following rules:

- Device names must not be empty
- SSH public keys must not be empty
- IPs must not be localhost (127.0.0.1, ::1, localhost)
- Ports must be valid (1-65535)

## Connection Rules

### Naming

Connection files use a directional naming scheme: `<local_user>-><remote_user>@<remote_device>.json`

The arrow indicates the direction of the connection from the perspective of the local device.

### Ownership

- A user may only modify connections where their name appears as the local member (before the arrow)
- This is enforced programmatically by `config::connection::{save,delete}_connection`
- Not just a convention — the functions check and reject invalid modifications

## Example Usage

```rust
use crate::config::{load_config, Device};

let conf = load_config();
for device in &conf.devices {
    println!("Device: {} ({})", device.name, device.ssh_public_key);
}
```

## Tests

Each behavior rule in the storage/git layer has a corresponding unit test in the same file under `#[cfg(test)]`.

Run tests with:
```bash
cargo test
```
