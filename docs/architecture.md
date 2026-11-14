# Architecture

> **Target of the upcoming refactor.** This page describes the target crate
> layout, layering, and interfaces. The current code is still a single binary
> crate with a `cli` module and the full CLI subcommands; it is being migrated
> to the structure described here (same convention as
> [Navigation](navigation.md): the docs are the spec, the code follows).

## Crate Layout and Layering (target)

The package builds **two crates from the same source tree**:

- **`regroup` library** (`src/lib.rs`) — everything reusable, with no
  terminal front-end:
  - `logging` — the UI-agnostic global log sink (`log`, `log_device`,
    `pulse_device`, `restore`, direct-output mode, `Level`/`UiEvent`).
    Until a front-end installs its own sink, messages go to stdout.
  - `config`, `discovery`, `git`, `ssh`, `signing` — the data and protocol
    layers, reporting through `logging`.
  - `ops` — the **feature layer**: every user-facing capability, callable
    from the TUI, from a future CLI, or from any Rust program embedding the
    library (see [modules/ops.md](modules/ops.md)). `ops` never depends on a
    front-end and never calls `process::exit`: it reports through `logging`
    and returns `Result`s.

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
           └── logging, config, discovery, git, ssh, signing
```

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

1. **Announce** itself every `wait_secs` on all interfaces (signed UDP beacon: name, SSH host public key, signing key, port — no user data)
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
- UDP-based discovery on a configurable port (default: 9876)
- Beacons are signed to prevent spoofing
- Contains: device name, SSH public key, signing public key, port, IP
- Beacon age is checked to filter out stale announcements

#### Configuration Storage
- Central git repository at `~/.regroup-config/git/`
- Each device has its own directory: `devices/<name>/`
- Decentralized: every device can push/pull from every other device
- Last-writer-wins for conflict resolution
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
├── git/                     # Source of truth, synced with peers via git
│   ├── .git/                # Git repository
│   ├── settings.json        # Global settings (udp_port, wait_secs)
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
