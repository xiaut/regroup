# Development Guide

This document describes how to develop and contribute to regroup.

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

### Continuous Interactive Mode

```bash
cargo run
```

Or with the built binary:
```bash
./target/debug/regroup
```

This starts the interactive mode, listening for beacons and prompting for new devices.

### Dry Run

```bash
cargo run -- test
```

Dry run mode: nothing is modified. Useful for testing without side effects.

### Self-Test

```bash
cargo run -- selftest
```

Runs built-in checks:
- External tool availability
- SSH keygen fingerprint parity

Exit code: 0 = success, 1 = failure

### Device Removal

```bash
cargo run -- remove <name>
```

Removes a device from the configuration.

### Full Wipe

```bash
cargo run -- wipe [--yes]
```

Deletes the entire regroup configuration. Use `--yes` to skip confirmation.

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

### Keep Files Short

When a file grows beyond ~500-800 lines, split it into a folder-module:

```
foo.rs → foo/mod.rs + foo/one.rs + foo/two.rs
```

**Exception**: `#[cfg(test)]` tests at the bottom of a file are exempt. However long, they are always legitimate.

### Module Organization

- Module-level rules go in `//!` docs at the top of the file
- Example: See `config::connection` for module documentation
- Each module should have a clear purpose

### Error Handling

- **Interactive loop**: Log errors via `ui::log` (appears in Logs pane)
- **CLI subcommands**: Use `die` with exit code 2
- **Self-test failures**: Exit with code 1

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

- Use `ui::log()` for general messages
- Use `ui::log_device()` for device-specific messages
- Include context in log messages

### Git Commits

- Use descriptive commit messages
- Include the "why" not just the "what"
- Reference issues/PRs if applicable
- Follow [Conventional Commits](https://www.conventionalcommits.org/) style

## File Structure

```
src/
├── main.rs              # Startup: CLI parsing, identity detection, thread wiring
├── lib.rs               # Library exports (if applicable)
├── ui/
│   ├── mod.rs           # UI module root
│   ├── types.rs         # UI types
│   ├── logging.rs       # Logging functions
│   ├── interface.rs     # UI interface
│   ├── tty.rs           # Full-screen TUI
│   ├── plain.rs         # Plain fallback UI
│   └── render.rs        # Rendering functions
├── interactive/
│   ├── mod.rs           # Interactive module root
│   ├── state.rs         # State management
│   └── utils.rs         # Utility functions
├── config/
│   ├── mod.rs           # Config types and constants
│   ├── loading.rs       # Config loading
│   ├── device.rs        # Device file handling
│   ├── connection.rs    # Connection file handling
│   ├── users.rs         # User file handling
│   ├── settings.rs      # Settings handling
│   └── validation.rs    # Validation functions
├── discovery/
│   ├── mod.rs           # Discovery orchestration
│   ├── beacon.rs        # Beacon format and verification
│   ├── local.rs         # Local identity detection
│   └── net.rs           # Network utilities
├── git/
│   ├── mod.rs           # Git module root
│   ├── repo.rs          # Repository management
│   ├── remote.rs        # Remote management
│   └── sync.rs          # Sync operations
├── ssh/
│   ├── mod.rs           # SSH module root
│   ├── config.rs        # SSH config management
│   ├── keys.rs          # Key types
│   └── public_key.rs    # Public key handling
├── signing.rs           # Signing key management
└── tui/
    └── connections.rs   # Connections browser (view-only)
```

## Performance Considerations

- Beacons are sent every 2 seconds (configurable via `wait_secs`)
- Beacon verification includes signature checking and IP validation
- Git sync operations can be slow with many remotes
- UI rendering is optimized to minimize redraws

## Debugging

### Verbose Mode

Run with `-v` or `--verbose` flag:
```bash
cargo run -- -v
```

Or toggle at runtime with `v` key in interactive mode.

### Logging

All UI messages appear in the Logs pane. For debugging:
- Check the Logs pane for warning and error messages
- Security messages are highlighted in red
- Use verbose mode for additional details

### Dry Run

Test without modifying anything:
```bash
cargo run -- test
```

This simulates the full workflow but makes no changes.
