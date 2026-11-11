# Storage Rules

The configuration repository `~/.regroup-config/git/` is shared decentralized state. All devices share the same repository and sync changes via git.

## Repository Structure

```
~/.regroup-config/git/
├── settings.json              # Global settings
└── devices/
    └── <name>/
        ├── device.json         # Latest known state
        ├── users/
        │   └── <user>.json    # One file per known user
        └── connections/
            └── <id>.json      # Connection files
```

## Device State (device.json)

Each device file contains:
- `name`: Device name (must be unique)
- `ssh_public_key`: SSH server host key (unique identifier)
- `signing_public_key`: Ed25519 signing key (pinned at first acceptance)
- `user`: Primary user associated with this device
- `port`: SSH port (default: 22)

## Write Ownership

**Critical rule**: A device only writes inside `devices/<its own name>/`.

- `users/` and `connections/` files are **never** written by peers
- `device.json` is **only** written by the device itself
- This rule prevents concurrent modification conflicts and ensures data consistency

## Conflict Resolution

- **Last-writer-wins**: The most recent commit wins
- Per-device directories keep concurrent writers apart; conflicts are rare
- Users should commit frequently to minimize conflicts

## Lists as Directories

**Extensible collections must be one file per item** so that:
- Two machines (or two users of one machine) appending different items never conflict
- Each addition creates a separate file, avoiding merge conflicts

Example:
- Users: `devices/<name>/users/<user>.json` (one file per user)
- Connections: `devices/<name>/connections/<id>.json` (one file per connection)

## Connection Naming

Connection files use a directional naming convention:

```
devices/<name>/connections/<connection_id>.json
```

Where `<connection_id>` follows the pattern: `<local_user>-><remote_user>@<remote_machine>`

### Direction

The arrow (`->`) carries the real direction:
- **Outgoing**: `alice->bob@laptop.json` (from alice to bob on laptop)
- **Incoming**: `bob@laptop->alice.json` (from bob@laptop to alice)

### Ownership

A user may only modify connection files where their own name is the **local member** (before the arrow).

This is enforced by `config::connection::{save,delete}_connection`, not by convention.

## No Legacy Migration

The flat layout (`devices/<name>.json`) is **not supported**. Every device must have its own directory under `devices/`.

This is a fresh start; no backward compatibility is provided.

## Invariants

### Identity
- **SSH host key** is the unique identifier — never hostname or IP
- `SshPublicKey` / `SshPublicKeyFingerprint` newtypes exist to prevent confusion
- Hostnames change (Termux randomizes), IPs change

### Signing
- Beacons are **signed** with the device's Ed25519 signing key
- Signing key is pinned in `device.json` at first acceptance
- Different signing keys must be rejected (or confirmed by user on reinstall)

### Verification
- Host key verification happens **before** accepting a device
- A mismatch means spoofing/MITM and **aborts acceptance**
- Verification is performed by `ssh::public_key::verify_and_add_to_known_hosts`

### SSH Config
- `~/.ssh/config` is the **user's file**
- Only rewrite regroup-managed `Host` blocks
- Be **idempotent** — running the same operation multiple times produces the same result
- Never touch other blocks
