# ssh Module

The `ssh` module handles SSH key management and known_hosts verification.

## Overview

This module provides utilities for working with SSH keys, including:
- Type-safe wrappers for different key types
- SSH host key fingerprint computation
- Known hosts verification and management
- SSH config file manipulation

## Module Structure

```
ssh/
├── mod.rs         # Module exports
├── config.rs      # ~/.ssh/config Host-block update/remove
├── keys.rs        # Typed wrappers: SshPublicKey, SigningPublicKeyB64, etc.
└── public_key.rs  # Host key fingerprint, known_hosts verification
```

## Key Types

The `keys.rs` module defines newtypes for strong typing:

### `SshPublicKey`

Wrapper for SSH host public keys:

```rust
pub struct SshPublicKey(pub String);
```

This ensures SSH host keys cannot be confused with other string types.

### `SigningPublicKeyB64`

Wrapper for Ed25519 signing public keys (base64 encoded):

```rust
pub struct SigningPublicKeyB64(pub String);
```

### `SigningKey`

Wrapper for Ed25519 signing private keys:

```rust
pub struct SigningKey(pub String);
```

## Why Newtypes?

Semantically different values must not share a bare type. Using newtypes:

- Prevents accidental misuse of keys
- Makes the code self-documenting
- Catches bugs at compile time

Example:
```rust
// This won't compile — different types!
let ssh_key: SshPublicKey = ...;
let signing_key: SigningPublicKeyB64 = ...;
ssh_key == signing_key; // Compile error
```

## SSH Config Management

The `config.rs` module manages `~/.ssh/config`:

### `update_block()`

Updates or adds a Host block for a device:

```rust
pub fn update_block(
    ssh_config: &Path,
    name: &str,
    ip: &str,
    user: &str,
    port: u16,
    port_explicit: bool,
) -> Result<(), SshConfigError>
```

This function:
- Finds or creates the Host block for the device
- Sets the hostname, user, port, and other SSH options
- Is **idempotent** — running it multiple times produces the same result
- Preserves existing blocks for other hosts

### `remove_block()`

Removes a Host block:

```rust
pub fn remove_block(ssh_config: &Path, name: &str) -> Result<bool, SshConfigError>
```

Returns `true` if the block was removed, `false` if it didn't exist.

## Known Hosts Management

The `public_key.rs` module handles known_hosts verification:

### `compute_fingerprint_from_public_key()`

Computes the SHA256 fingerprint of an SSH public key:

```rust
pub fn compute_fingerprint_from_public_key(key: &str) -> Option<String>
```

### `verify_and_add_to_known_hosts()`

Verifies a host's key and adds it to known_hosts:

```rust
pub enum HostKeyVerification {
    Success,                    // Key verified and added
    HostUnreachable,           // Could not connect to host
    FingerprintMismatch {      // Key doesn't match expected
        expected: String,
        actual: String,
    },
    WriteError,                // Could not write to known_hosts
}

pub fn verify_and_add_to_known_hosts(
    ip: &str,
    port: u16,
    expected_fingerprint: &str,
) -> HostKeyVerification
```

This function:
1. Connects to the host at the given IP and port
2. Retrieves the host's SSH public key
3. Computes its fingerprint
4. Compares with the expected fingerprint
5. If match: adds to known_hosts and returns Success
6. If mismatch: returns FingerprintMismatch (SECURITY ISSUE!)

### `remove_known_host()`

Removes all entries for an IP from known_hosts:

```rust
pub fn remove_known_host(ip: &str) -> usize
```

Returns the number of lines removed.

## Local SSH Server Key

The `public_key.rs` module can detect the local SSH server key:

### `get_ssh_server_public_key()`

Extracts the local SSH server's public key:

```rust
pub fn get_ssh_server_public_key() -> Option<SshPublicKey>
```

This function tries:
1. Reading from `/etc/ssh/ssh_host_*_key.pub` files
2. Using `ssh-keyscan` to query localhost

Returns `None` if no SSH server key is found (e.g., no SSH server installed).

## Usage Example

```rust
use crate::ssh::{config::update_block, public_key::verify_and_add_to_known_hosts};

// Add a device to SSH config
update_block(
    &Path::new("/home/user/.ssh/config"),
    "my-server",
    "192.168.1.100",
    "user",
    22,
    false,
).unwrap();

// Verify a host's key
match verify_and_add_to_known_hosts("192.168.1.100", 22, "SHA256:abc123...") {
    HostKeyVerification::Success => println!("Host verified!"),
    HostKeyVerification::FingerprintMismatch { expected, actual } => {
        eprintln!("SECURITY: Fingerprint mismatch!");
        eprintln!("Expected: {}", expected);
        eprintln!("Actual: {}", actual);
    }
    _ => println!("Could not verify host"),
}
```

## Security Considerations

- **Always verify host keys** before accepting a device
- **Fingerprint mismatches** indicate possible spoofing/MITM — abort acceptance
- **known_hosts entries** are cleaned up when IPs change to prevent stale data
- **SSH config is the user's file** — only modify regroup-managed blocks
