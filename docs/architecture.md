# Architecture

> **Target of the upcoming refactor.** This page describes the target crate
> layout, layering, and interfaces. The current code is still a single binary
> crate with a `cli` module and the full CLI subcommands; it is being migrated
> to the structure described here (same convention as
> [Navigation](navigation.md): the docs are the spec, the code follows).

## Crate Layout and Layering (target)

The package builds **two crates from the same source tree**:

- **`regroup` library** (`src/lib.rs`) — everything reusable, with no
  terminal front-end. Two blocks, strictly separated:
  - **Data & protocol layers** — `config`, `discovery`, `git`, `ssh`,
    `signing`: types, on-disk formats, and wire protocols. They know
    nothing about user-facing features and report through `logging`.
  - **Feature layer** — `ops`: every user-facing capability, callable
    from the TUI, from a future CLI, or from any Rust program embedding the
    library (see [modules/ops.md](modules/ops.md)). `ops` sits on top of the
    data layers, never depends on a front-end, and never calls
    `process::exit`: it reports through `logging` and returns `Result`s.
  - `logging` — the UI-agnostic global log sink (`log`, `log_device`,
    `pulse_device`, `restore`, direct-output mode, `Level`/`UiEvent`),
    shared by both blocks. Until a front-end installs its own sink,
    messages go to stdout.

- **`regroup` binary** (`src/main.rs`, thin) — argument dispatch, app wiring
  (identity, beacon threads), and the front-ends:
  - `internal` — the `receive-install-key` / `receive-remove-key`
    subcommands, run on the target device over SSH by the key actions (see
    [modules/internal.md](modules/internal.md)).
  - `ui`, `tui`, `interactive` — the terminal front-end (full-screen TUI with
    automatic plain fallback, key actions, main loop).

Dependency direction is strictly downward:

```
main.rs (binary)
 └── internal, ui, tui, interactive     front-ends
      └── ops                         features
           └── logging, config, discovery, git, ssh, signing   data & protocol
```

### File Layout: one file per responsibility

The library tree is deliberately **fine-grained** — one file per
responsibility — so the repository can be understood by reading many small
files instead of a few huge ones:

```
src/ (library part)
├── lib.rs            # crate root: declares the modules below
├── logging.rs        # shared log sink (used by the data layers and by ops)
├── config/           # data layer: ~/.regroup-config state — modules/config.md
│   ├── mod.rs        #   types, storage constants, path helpers
│   ├── proxy.rs      #   git/ + tmp/ merged reads, lazy purge, deletes in both layers
│   ├── loading.rs    #   load_config(): ensure repo, scan devices/ through the proxy
│   ├── device.rs     #   device.json load/save
│   ├── users.rs      #   users/<user>.json — one file per known user
│   ├── connection.rs #   connections/<id>.json — naming + ownership rules
│   ├── name.rs       #   validated device-name newtype
│   ├── settings.rs   #   local settings.json (outside git/, never synced)
│   ├── validation.rs #   device sanity checks
│   └── json_helpers.rs
├── discovery/        # data/protocol: signed UDP beacons — modules/discovery.md
│   ├── mod.rs
│   ├── beacon.rs     #   beacon format, signing, verification
│   ├── local.rs      #   local identity: hostname, IP, host key
│   └── net.rs        #   UDP send/receive helpers
├── git/              # data layer: config repo, remotes, sync — modules/git.md
│   ├── mod.rs
│   ├── repo.rs       #   repo init + minimal repo-local git config
│   ├── remote.rs     #   per-device remotes
│   └── sync.rs       #   pull/push, commit helpers
├── ssh/              # data/protocol: ssh config + keys — modules/ssh.md
│   ├── mod.rs
│   ├── config.rs     #   managed Host blocks in ~/.ssh/config
│   ├── keys.rs       #   newtypes (SshPublicKey, SigningPublicKeyB64, ...)
│   └── public_key.rs #   fingerprints, known_hosts verification
├── signing.rs        # Ed25519 signing key management
└── ops/              # feature layer, one file per capability — modules/ops.md
    ├── mod.rs
    ├── devices.rs    #   add / remove / wipe devices, name conflicts
    ├── sync.rs       #   git sync of the config repo, signing-key trust
    ├── keys.rs       #   connection key pairs, authorized_keys, connection IDs
    ├── shell.rs      #   setup_cd (regroup-cd shell function)
    ├── selftest.rs   #   environment checks, returns a report (no exit code)
    └── help.rs       #   help text shared with the TUI '?' panel
```

The binary half of the tree (`main.rs`, `internal/`, `ui/`, `tui/`,
`interactive/`) is listed in [File Structure](development.md#file-structure).

Splitting rules:

- A module is a **directory** with a `mod.rs` that declares and re-exports
  its submodules; each file covers a single topic.
- Keep files short: past ~500-800 lines (excluding `#[cfg(test)]` tests,
  which stay at the bottom of their file), split `foo.rs` into a
  folder-module: `foo.rs → foo/mod.rs + foo/one.rs + foo/two.rs`.
- Every module has a matching spec in `docs/modules/<name>.md`; its
  "Module Structure" section lists the same files — the code tree and the
  docs must stay in sync.
- Where the current tree already exceeds the limit (e.g. `ui/render.rs`,
  `interactive/state.rs`), the migration splits those files the same way.

## Interfaces (target)

- `regroup` with **no arguments** starts the interactive TUI. When stdout is
  not a terminal, the plain line-based fallback is selected automatically.
- The full CLI subcommands — `test`, `remove <name>`, `wipe [--yes]`,
  `setup-cd`, `selftest`, and the `-h` / `--cli` / `--yes` flags — are
  **dropped from the binary**. Their logic lives on in the library:
  `ops::devices::{remove_device, remove_local_traces, wipe}`,
  `ops::shell::setup_cd`, `ops::selftest::run` (returns a report instead of
  exiting), and dry-run remains a capability of the interactive front-end
  (`Context::dry_run`), e.g. for tests. Help is shown in the TUI with `?`.
- `regroup receive-install-key ...` and `regroup receive-remove-key ...`
  remain as **internal** subcommands: they are executed on the target device
  via `ssh user@device regroup receive-...` by the TUI key actions, and are
  not meant for direct use.
- Exit codes: `0` normal exit, `2` configuration or environment error
  (internal subcommands: `0` success, `1` handler error).

## Big Picture

A **regroup** process does three things simultaneously:

1. **Announce** itself every 2 s on all interfaces, for a window of `wait_secs` (signed UDP beacon: name, SSH host public key, signing key, SSH port — no user data)
2. **Listen** for other devices' beacons; unknown devices appear in the interactive UI's "Detected" panel and are added on acceptance
3. **Sync** its configuration with peers through the git repository in `~/.regroup-config/git/` (remotes are auto-configured per known device)

### Data Flow

```
                   UDP beacon (signed, ~2 s)
  device A  ┌─────────────────────────────────►  device B
     │      │                                  │
     │      │  announce self + listen           │  listen + prompt user
     ▼      ▼                                  ▼
  ~/.regroup-config/git/   ◄── git sync ──►  ~/.regroup-config/git/
  (shared config repo)        (pull/push)      (shared config repo)
     │                                           │
     ▼                                           ▼
  ~/.ssh/config (one "Host" block per device, regenerated idempotently)
```

### Key Concepts

#### Device Identity
- **SSH host key** is the unique identifier for a device (never hostname or IP, which can change)
- Each device has an Ed25519 **signing key** used to sign beacons
- Signing keys are pinned at first acceptance; different keys trigger a rotation prompt

#### Beacon Protocol
- UDP broadcast discovery on a single configurable port (`udp_port`, default:
  4242) used for **both** listening and announcing — like a channel: the listener
  binds `0.0.0.0:udp_port`, the announcer sends to `<interface-broadcast>:udp_port`.
  All devices on the LAN must use the same port to hear each other; it is agreed
  out-of-band (default or local `settings.json`), never carried in the beacon —
  the beacon's `port` field is the device's **SSH** port
- Beacons are signed to prevent spoofing
- Contains: device name, SSH public key, signing public key, SSH port, IP
- Beacon age is checked to filter out stale announcements

#### Configuration Storage
- Central git repository at `~/.regroup-config/git/`
- Each device has its own directory: `devices/<name>/`
- Decentralized: every device can push/pull from every other device
- Every file in the repo has a single writer (its device), so conflicts cannot
  occur; first sync between bootstrapped repos uses rebase (unrelated histories)
- Local settings (`udp_port`, `wait_secs`) live in `~/.regroup-config/settings.json`, outside the repo
- **Write isolation**: Each device only writes its own files into `git/`; accepting a device adds a git remote and pulls its authoritative data. Beacon-derived placeholders and temporary users go to the `tmp/` overlay and are never synced; they are purged once the authoritative files arrive in `git/` (see [Storage Rules](storage.md#configuration-proxy-the-tmp-overlay))

## Thread Model

regroup uses a multi-threaded architecture:

- **Main thread**: Interactive UI loop (handles user input, displays state)
- **Listener thread**: Receives and verifies incoming beacons
- **Announcer thread**: Periodically broadcasts this device's beacon

Threads communicate via channels (MPSC for beacon notifications, UI events).

## Security Model

1. **Signing key pinning**: A device's Ed25519 signing key is pinned when first accepted. Beacons with different keys are rejected (or trigger rotation prompt on reinstall).

2. **Host key verification**: Before accepting a device, regroup verifies that the announced IP serves the expected SSH host key (fingerprint match). Mismatches abort acceptance (possible spoofing/MITM).

3. **Write ownership**: A device only writes inside `devices/<its own name>/`. Other devices' files are never modified directly — changes propagate via git sync.

4. **SSH config**: Only regroup-managed `Host` blocks are rewritten; existing blocks are never touched. Changes are idempotent.

## Storage Layout

```
~/.regroup-config/
├── signing/
│   └── private_key          # This device's Ed25519 signing key
├── settings.json            # Local-only settings (udp_port, wait_secs) — never synced
├── git/                     # Source of truth, synced with peers via git: devices/ only
│   ├── .git/                # Git repository
│   └── devices/
│       └── <device-name>/
│           ├── device.json          # Device metadata (name, keys, port)
│           ├── users/
│           │   └── <user>.json      # Known users on this device
│           └── connections/
│               └── <id>.json      # Connection files
├── tmp/                     # Temporary overlay — never committed, never synced
│   └── devices/<name>/      # not-yet-authoritative data about another device
│       ├── device.json      #   e.g. placeholder for an accepted device
│       └── users/
│           └── <user>.json  #   e.g. temporary user pending confirmation
└── .gitignore
```

See [Storage Rules](storage.md) for detailed constraints.
