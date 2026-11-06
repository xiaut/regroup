# regroup — developer notes

**Note**: This document has been moved to the [`docs/`](../docs/) directory for better organization.
Please see the following documentation files:

- [docs/README.md](../docs/README.md) — Documentation overview and table of contents
- [docs/architecture.md](../docs/architecture.md) — Big picture, data flow, and how regroup works
- [docs/storage.md](../docs/storage.md) — Storage rules, write ownership, and conflict resolution
- [docs/modules/](../docs/modules/) — Module-specific documentation
- [docs/development.md](../docs/development.md) — Coding rules and development guide

User-facing documentation lives in the [root README](../README.md).

This document is kept for backwards compatibility but may be removed in a future version.

## Big picture

```
                   UDP beacon (signed, ~2 s)
  device A  ─────────────────────────────────►  device B
     │                                             │
     │  announce self + listen                     │  listen + prompt user
     ▼                                             ▼
  ~/.regroup-config/git/   ◄── git sync ──►  ~/.regroup-config/git/
  (shared config repo)        (pull/push)      (shared config repo)
     │                                             │
     ▼                                             ▼
  ~/.ssh/config (one "Host" block per device, regenerated idempotently)
```

A regroup process does three things at once:

1. **Announce** itself every `wait_secs` on all interfaces (signed UDP
   beacon: name, SSH host public key, signing key, user, port).
2. **Listen** for other devices' beacons; unknown devices appear in the
   interactive UI's "Detected" panel and are added on acceptance.
3. **Sync** its configuration with peers through the git repository in
   `~/.regroup-config/git/` (remotes are auto-configured per known device).

## Module map

```
src/
├── main.rs             startup: CLI parsing, identity detection, thread wiring
├── ui.rs               interactive UI layer: full-screen TUI (ratatui over
│                       crossterm) or plain fallback; owns key→Action mapping
├── interactive.rs      interactive loop: consumes Actions, mutates state
│                       (pending devices, known machines, panels)
├── signing.rs          Ed25519 signing key pair (under ~/.regroup-config/signing/)
├── discovery/
│   ├── beacon.rs       beacon format, signing + verification, trusted-key checks
│   ├── local.rs        local identity: hostname, IP, SSH server key detection
│   └── mod.rs          discovery orchestration
├── config/
│   ├── mod.rs          Machine/Conf types, storage constants, path helpers
│   ├── loading.rs      load_config(): ensure repo, scan devices/,
│   │                   auto-create local device, record user
│   ├── device.rs       device file (device.json) load/save + validation tests
│   ├── connection.rs   connection files: naming rule, ownership enforcement,
│   │                   loader (see "Storage rules" below) — create/remove
│   │                   procedures not implemented yet
│   ├── users.rs        users/<user>.json: one file per known user
│   ├── settings.rs     settings.json (udp_port, wait_secs)
│   ├── validation.rs   Machine sanity checks (localhost, empty keys, ...)
│   └── json_helpers.rs int parsing helper
├── git/
│   ├── repo.rs         git init + repo checks (updateInstead, pull.rebase)
│   ├── remote.rs       remote add/remove/list, remotes sync per device
│   └── sync.rs         pull/push all, commit helper, git_is_tracked
├── ssh/
│   ├── config.rs       ~/.ssh/config Host-block update/remove (idempotent)
│   ├── keys.rs         typed wrappers: SshPublicKey, SigningPublicKeyB64, ...
│   │                   (newtypes, never confuse host keys with signing keys)
│   └── public_key.rs   host key fingerprint, known_hosts verification
└── tui/
    └── connections.rs  connections browser (view-only; not wired into the
                        main loop yet)
```

## Storage rules (do not break)

The config repository `~/.regroup-config/git/` is shared decentralized
state:

```
settings.json
devices/
  <name>/
    device.json              latest known state (name, ssh_public_key,
                             signing_public_key, user, port)
    users/<user>.json        one file per user known on the device
    connections/<conn>.json connection files (create/remove TBD)
```

- **Write ownership**: a device only writes inside `devices/<its own name>/`.
  `users/` and `connections/` files are never written by peers. Exception:
  `device.json` is also written by a device that discovers new knowledge
  about a peer (acceptance), otherwise git remotes could never be
  bootstrapped.
- **Conflicts**: the most recent commit wins (last-writer-wins). Per-device
  directories keep concurrent writers apart; conflicts are already rare.
- **Lists are directories**: extensible collections must be one
  file per item so two machines (or two users of one machine) appending
  different items never conflict.
- **Connection naming**: inside `devices/<name>/connections/`, the member
  on `<name>` is a bare user name, the other member is `user@machine`, and
  the arrow carries the real direction: outgoing
  `alice->bob@laptop.json`, incoming `bob@laptop->alice.json`. A user may
  only modify connection files where their own name is the local member;
  this is enforced by `config::connection::{save,delete}_connection`, not
  by convention.
- **No legacy migration**: the flat layout (`devices/<name>.json`) is
  not supported. Every device must have its own directory under `devices/`.
  This is a fresh start; no backward compatibility is provided.

## Invariants

- Identity is the **SSH host key**: never identify a device by hostname or
  IP. Hostnames change (Termux randomizes), IPs change. `SshPublicKey` /
  `SshPublicKeyFingerprint` newtypes exist so they cannot be confused.
- Beacons are **signed**: a device's Ed25519 signing key is pinned in its
  device.json at first acceptance; other keys must be rejected (or, on
  reinstall, confirmed by the user). See `discovery::beacon`.
- Host key verification happens **before** accepting a device
  (`ssh::public_key::verify_and_add_to_known_hosts`): a mismatch means
  spoofing/MITM and aborts acceptance.
- `~/.ssh/config` is the **user's file**: only rewrite regroup-managed
  `Host` blocks, be idempotent, never touch other blocks.

## Build & test

```bash
cargo build                 # debug binary
cargo test                  # unit tests (config, beacons, naming rules, ...)
cargo run                   # continuous interactive mode
cargo run -- test           # dry run
cargo run -- selftest       # checks external tools + fingerprint parity
cargo run --release         # release build
```

Rust edition 2024, no async runtime; threads + channels for the beacon
listener/announcer.

## Coding rules

- **No backward compatibility** — formats, APIs and behavior may
  change between versions; do not add compat shims, deprecated aliases or
  dual code paths. The on-disk layout may evolve; old data will not be
  supported.
- **Newtypes for strong typing**: wrap primitives in one-field tuple
  structs (`pub struct SshPublicKey(pub String);`). Semantically different
  values must not share a bare type. The field may be public — no need to
  re-expose the inner type's methods — unless construction has invariants
  to check. See `ssh/keys.rs` for the pattern.
- **Keep files short**: when a file grows, split it into a folder-module
  with file-submodules (`foo.rs` → `foo/mod.rs` + `foo/one.rs`,
  `foo/two.rs`). `#[cfg(test)]` tests at the bottom of a file are exempt:
  however long, they are always legitimate.

## Conventions

- English everywhere, including docs, logs, and commit messages.
- Module-level rules go in `//!` docs at the top of the file (see
  `config::connection` for an example).
- Errors in the interactive loop are logged via `ui::log` so they appear in
  the activity pane; CLI subcommands `die` with exit code 2; `selftest`
  failures exit 1.
- Each behavior rule added to the storage/git layer gets a unit test in the
  same file under `#[cfg(test)]`.
