# Development Guide

This document describes how to develop and contribute to regroup.

> **Target of the upcoming refactor.** The file structure, running, and error
> handling sections below describe the target layout — a `regroup` library
> crate (`src/lib.rs`) with the feature layer in `ops`, and a thin binary
> (`src/main.rs`) that keeps the TUI and the internal `receive-*`
> subcommands only. Where the current code differs, this guide wins; the code
> is being migrated to it (same convention as [navigation.md](navigation.md)).

## Prerequisites

### Required Tools

- **Rust**: The project is written in Rust. Install via rustup:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- **Git**: For version control and configuration sync

- **SSH tools**: `ssh-keyscan` and `ssh-keygen` are required for key verification
  - Termux: `pkg install openssh`
  - Ubuntu: `sudo apt install openssh-client`

### External Dependencies

The following system packages are required:

- `openssh-client` (for `ssh-keyscan`, `ssh-keygen`)
- `git` (for configuration sync)

## Building

### Debug Build

```bash
cargo build
```

This creates a debug binary at `target/debug/regroup`.

### Release Build

```bash
cargo build --release
```

This creates an optimized binary at `target/release/regroup`.

## Running

The binary has a single interface: the interactive TUI.

```bash
cargo run
```

Or with the built binary:
```bash
./target/debug/regroup
```

This starts the interactive mode (full-screen TUI when stdout is a terminal,
plain line-based fallback otherwise), listening for beacons and prompting for
each new device. Any other command-line arguments are rejected with exit code
2 — except the internal `receive-install-key` / `receive-remove-key`
subcommands, which regroup runs on a target device over SSH (see
[modules/internal.md](modules/internal.md)).

> The former subcommands (`test`, `remove <name>`, `wipe`, `setup-cd`,
> `selftest`) and the `-h` / `--cli` / `--yes` flags are **dropped from the
> binary** in the target interface. Their logic lives on in the library:
> `regroup::ops::devices::{remove_device, wipe}`, `regroup::ops::shell::setup_cd`,
> `regroup::ops::selftest::run` (returns a report, no exit code). Dry-run is
> no longer a flag; it remains a `Context::dry_run` capability of the
> interactive front-end (used by tests). Until the refactor lands, the current
> binary still accepts the old arguments.

## Using Regroup as a Library

The package also builds a library crate named `regroup` (`src/lib.rs`). All
features live in the UI-agnostic `ops` layer and can be driven from the TUI,
a CLI, or any Rust program:

```rust
use regroup::ops;

// Environment checks (external tools, ssh-keygen fingerprint parity)
let report = ops::selftest::run();
if !report.ok() {
    // report.tools_ok / report.fingerprint_ok
}

// Sync the config repository with all known devices
ops::sync::sync_git(&home, &temp_users);
```

`ops` never touches the terminal directly and never calls `process::exit`:
it reports through the `regroup::logging` sink (stdout until a front-end
installs its own) and returns `Result`s, so the caller decides how to present
failures. See [ops module](modules/ops.md) and
[Architecture](architecture.md#crate-layout-and-layering-target).

## Testing

### Unit Tests

```bash
cargo test
```

Runs all unit tests. Tests cover:
- Configuration loading and validation
- Beacon signing and verification
- Naming rules
- Connection file handling
- Git operations (if git is available)

### Integration Tests

Some tests require external tools (git, ssh-keyscan). These are skipped if the tools are not available.

### Test Structure

Each behavior rule in the storage/git layer has a corresponding unit test in the same file under `#[cfg(test)]`.

Example:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_device_naming() {
        // Test implementation
    }
}
```

## Coding Rules

### No Backward Compatibility

- Formats, APIs, and behavior **may change** between versions
- Do **not** add compatibility shims
- Do **not** add deprecated aliases
- Do **not** implement dual code paths
- Old on-disk data will not be supported

### Newtypes for Strong Typing

**Always** wrap primitives in one-field tuple structs:

```rust
// Good
pub struct SshPublicKey(pub String);
pub struct SigningPublicKeyB64(pub String);

// Bad
pub type SshPublicKey = String;
```

**Why?**
- Semantically different values must not share a bare type
- Prevents bugs at compile time
- Makes code self-documenting

**When the field can be public:**
- No invariants to check
- No need to re-expose the inner type's methods

**When to use private field:**
- Construction has invariants to check
- Need to enforce constraints on the value

See `ssh/keys.rs` for examples of this pattern.

### Keep Files Short — one file per responsibility

The repository is meant to be understood file by file: each module is a
directory whose files each cover a single topic, so no reader has to digest
a huge file to understand one behavior.

When a file grows beyond ~500-800 lines, split it into a folder-module:

```
foo.rs → foo/mod.rs + foo/one.rs + foo/two.rs
```

**Exception**: `#[cfg(test)]` tests at the bottom of a file are exempt. However long, they are always legitimate.

Each module's files are listed in its spec (`docs/modules/<name>.md`,
"Module Structure" section) and in the [File Structure](#file-structure)
tree below — keep all three in sync when adding, removing or splitting a
file. See [Architecture](architecture.md#crate-layout-and-layering-target)
for the layering rules (ops feature layer vs data layers).

### Module Organization

- Module-level rules go in `//!` docs at the top of the file
- Example: See `config::connection` for module documentation
- Each module should have a clear purpose

### Error Handling

- **Feature layer (`ops`, library)**: return `Result<_, String>` and report
  through the `regroup::logging` sink; never call `process::exit` — the
  caller (binary, TUI, lib consumer) decides how to report and which exit
  code to use
- **Interactive loop (front-end)**: log errors via the logging sink
  (Logs pane in the full-screen UI, stdout in plain mode)
- **Binary (`main.rs`)**: `die` with exit code 2 for configuration or
  environment errors; the internal `receive-*` subcommands exit 1 on handler
  error, 0 on success. Selftest is a library feature (`ops::selftest::run`)
  returning a report — it has no exit code

### Documentation

- **English everywhere**: Docs, logs, and commit messages must be in English
- Use `//!` for module-level documentation
- Use `///` for item-level documentation
- Document invariants, not just what the code does

## Conventions

### Naming

- Use descriptive names for types and functions
- Prefer clarity over brevity
- Follow Rust naming conventions (snake_case for functions, PascalCase for types)

### Logging

- The sink is `regroup::logging` (library): `log()` for general messages,
  `log_device()` for device-specific messages; front-ends re-export it
  (`ui::log`, `ui::log_device`)
- `ops` and the data modules report through the same sink — never print
  directly
- Include context in log messages
- There is no verbose mode. Follow the [Logging policy](logging.md): log a
  line just before any operation that may prompt for a password, and log
  every non-standard path (fallbacks, dry-run skips, warnings, errors,
  security events, no-ops)

### Git Commits

- Use descriptive commit messages
- Include the "why" not just the "what"
- Reference issues/PRs if applicable
- Follow [Conventional Commits](https://www.conventionalcommits.org/) style

## File Structure

Two crates from one source tree: a library (`src/lib.rs`) and a thin binary
(`src/main.rs`). Shared modules are declared once, by the library.

The tree below is split **one file per responsibility** (see
[Keep Files Short](#keep-files-short--one-file-per-responsibility)): each
module is a directory of small files, and each module's spec in
`docs/modules/` lists the same files — so the repository can be understood
without reading huge files.

```
src/
├── lib.rs               # Library crate root: pub mod logging, config, discovery,
│                        #   git, ssh, signing, ops
├── main.rs              # Binary: thin arg dispatch (TUI + internal receive-*), app
│                        #   wiring (identity, beacon threads)
├── logging.rs           # UI-agnostic global log sink (log, log_device, pulse_device,
│                        #   restore; Level, UiEvent) — library module, re-exported by ui
├── config/              # Data layer: config types, loading, storage — modules/config.md
│   ├── mod.rs           #   types, storage constants, path helpers
│   ├── proxy.rs         #   git/ + tmp/ merged reads, lazy purge, deletes in both layers
│   ├── loading.rs       #   load_config(): ensure repo, scan devices/ through the proxy
│   ├── device.rs        #   device.json load/save
│   ├── users.rs         #   users/<user>.json — one file per known user
│   ├── connection.rs    #   connections/<id>.json — naming + ownership rules
│   ├── name.rs          #   validated device-name newtype
│   ├── settings.rs      #   local settings.json (outside git/, never synced)
│   ├── validation.rs    #   device sanity checks
│   └── json_helpers.rs  #   internal JSON parsing helpers
├── discovery/           # Data/protocol: signed UDP beacons — modules/discovery.md
│   ├── mod.rs
│   ├── beacon.rs        #   beacon format, signing, verification
│   ├── local.rs         #   local identity: hostname, IP, host key
│   └── net.rs           #   UDP send/receive helpers
├── git/                 # Data layer: repo, remotes, sync — modules/git.md
│   ├── mod.rs
│   ├── repo.rs          #   repo init + minimal repo-local git config
│   ├── remote.rs        #   per-device remotes
│   └── sync.rs          #   pull/push, commit helpers
├── ssh/                 # Data/protocol: ssh config blocks, key types, public keys — modules/ssh.md
│   ├── mod.rs
│   ├── config.rs        #   managed Host blocks in ~/.ssh/config
│   ├── keys.rs          #   newtypes (SshPublicKey, SigningPublicKeyB64, ...)
│   └── public_key.rs    #   fingerprints, known_hosts verification
├── signing.rs           # Library: Ed25519 signing key management
├── ops/                 # Feature layer (library API, no front-end deps) — modules/ops.md
│   ├── mod.rs
│   ├── devices.rs       # add/remove/wipe device, local traces, name-conflict helpers
│   ├── sync.rs          # sync_git, trust_new_signing_key
│   ├── keys.rs          # connection key pairs, authorized_keys, connection IDs
│   ├── shell.rs         # setup_cd / shell_rc_file (regroup-cd shell function)
│   ├── selftest.rs      # environment selftest as a lib feature (returns a report)
│   └── help.rs          # help text shared with the TUI '?' panel
├── internal/            # Binary: internal subcommands run over SSH by the key actions
│   ├── mod.rs
│   ├── receive_install_key.rs
│   └── receive_remove_key.rs
├── ui/                  # Binary front-end: Tty/Plain backends — modules/ui.md
│   ├── mod.rs           #   (re-exports regroup::logging)
│   ├── types.rs         #   UI types (re-exports Level, UiEvent from regroup::logging)
│   ├── interface.rs     #   Ui enum (Tty/Plain)
│   ├── tty.rs           #   full-screen TUI
│   ├── plain.rs         #   plain fallback
│   ├── render.rs        #   rendering
│   └── keys.rs          #   single-source key dispatch (bindings/hints from one list)
├── tui/                 # Binary front-end: key actions, connections browser
│   ├── mod.rs
│   ├── key_actions.rs   #   install/remove SSH key via ssh receive-*
│   └── connections.rs   #   connections browser
└── interactive/         # Binary front-end: main loop, state — modules/interactive.md
    ├── mod.rs
    ├── state.rs
    └── utils.rs
```

## Performance Considerations

- Beacons are sent every 2 seconds; the announce window is `wait_secs` and the UDP channel port is `udp_port` (default 4242), both from local `settings.json`
- Beacon verification includes signature checking and IP validation
- Git sync operations can be slow with many remotes
- UI rendering is optimized to minimize redraws

## Debugging

### Logging

All UI messages appear in the Logs pane. For debugging:
- Check the Logs pane for warning and error messages
- Security messages are highlighted in red
- There is no verbose mode; see the [Logging policy](logging.md) for what
  must be logged and when

### Dry Run

Dry run is no longer a binary flag in the target interface. It remains a
capability of the interactive front-end (`interactive::Context::dry_run`):
when set, no files are modified and no git operations run, and every skipped
action is logged (`[DRY RUN] ...` / `  - (dry run - ... skipped)`). Front-ends
and tests can enable it; the shipped binary does not.
