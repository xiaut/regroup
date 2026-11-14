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
- `port`: SSH port (default: 22)

## Write Ownership

**Critical rule**: A device only writes inside `devices/<its own name>/`.

- `users/` and `connections/` files are **never** written by peers
- `device.json` in `git/` is **only** written by the device itself
- This rule prevents concurrent modification conflicts and ensures data consistency
- Data about *other* devices that this device is not authoritative for (e.g. the
  beacon-derived placeholder created at acceptance time) never enters `git/` —
  it goes to the `tmp/` overlay (see below) and is never synced

## Configuration Proxy: the `tmp/` Overlay

All configuration access goes through a proxy that merges two layers:

- `~/.regroup-config/git/` — the git repository: **source of truth**, synced with peers
- `~/.regroup-config/tmp/` — a plain directory (a sibling of `git/`, never inside
  the repository, so it can never be committed, pushed, or pulled) holding
  **temporary, not-yet-authoritative data**

`tmp/` mirrors the `git/` layout (`tmp/devices/<name>/device.json`,
`tmp/devices/<name>/users/<user>.json`, ...).

**All state lives on disk**: `git/` + `tmp/` together are the complete
configuration. In-memory structures (device list, user maps, ...) are only a
cache of the merged proxy view, never a separate source of truth.

### Write Rule

- Authoritative data — this device's own `devices/<self>/` and `settings.json` —
  is written to `git/` and synced, as before.
- Temporary data about **other** devices is written to `tmp/` instead of `git/`:
  - the placeholder `device.json` written from beacon data when a device is
    accepted, before its authoritative file can be pulled
    (`tmp/devices/<name>/device.json`)
  - the temporary user added in the Users panel
    (`tmp/devices/<name>/users/<user>.json`)

### Read Rule (Merge)

Reading the configuration merges the two layers per file:

```
effective = git/ ∪ (tmp/ − git/)
```

On overlap, `git/` wins: the `tmp/` copy is stale (its authoritative counterpart
arrived via sync) and **must be removed**. `tmp/` therefore only ever contributes
paths that do not exist in `git/` yet — it can never override synced data.

### Purge Rule

A `tmp/` file whose relative path also exists in `git/` is stale and is removed:

- **lazily on read** (self-healing), and
- **eagerly after each successful git sync** (keeps `tmp/` small even if nothing
  reads that path).

Purge is per file, not per directory: `tmp/devices/X/` keeps its non-overlapping
files while `tmp/devices/X/device.json` is dropped once `git/devices/X/device.json`
exists. Same for temporary users: `tmp/devices/X/users/alice.json` is dropped once
`git/devices/X/users/alice.json` arrives via sync.

### Delete Rule

Removing a device (or any file) deletes the path in **both** layers — otherwise a
stale `tmp/` placeholder would resurrect a removed device on the next read.

### Concurrency

regroup is multi-threaded (UI + beacon listener). A proxy read snapshots both
layers and merges them in memory; the lazy purge runs under the same lock as the
read that triggers it.

### Placement & Permissions

`tmp/` lives at `~/.regroup-config/tmp/` and is created with mode `0700` (it may
hold not-yet-verified data).

### No Migration

Placeholder `device.json` files already committed in `git/` by older versions
remain in place (source of truth) until the device's own file arrives via sync and
wins last-writer-wins.

## Conflict Resolution

- **Last-writer-wins**: The most recent commit wins
- Per-device directories keep concurrent writers apart; conflicts are rare
- Users should commit frequently to minimize conflicts

## Lists as Directories

**Extensible collections must be one file per item** so that:
- Two devices (or two users of one device) appending different items never conflict
- Each addition creates a separate file, avoiding merge conflicts

Example:
- Users: `devices/<name>/users/<user>.json` (one file per user)
- Connections: `devices/<name>/connections/<id>.json` (one file per connection)

## Connection Naming

Connection files use a directional naming convention:

```
devices/<name>/connections/<connection_id>.json
```

Where `<connection_id>` follows the pattern: `<local_user>-><remote_user>@<remote_device>`

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
