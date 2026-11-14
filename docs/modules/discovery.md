# discovery Module

The `discovery` module handles device detection via UDP beacons.

## Overview

This module implements the core discovery mechanism: devices announce themselves via signed UDP beacons, and regroup listens for these announcements to detect new devices on the local network.

## Module Structure

```
discovery/
├── mod.rs          # Discovery orchestration
├── beacon.rs       # Beacon format, signing + verification, trusted-key checks
├── local.rs        # Local identity: hostname, IP, SSH server key detection
└── net.rs          # Network utilities for beacon transmission
```

## Beacon Format

A beacon contains the following information:

```rust
pub struct BeaconDevice {
    pub name: String,
    pub ssh_public_key: SshPublicKey,
    pub signing_public_key: SigningPublicKeyB64,
    pub ip: String,
    pub port: u16,
}
```

The beacon is signed with the device's Ed25519 signing key to ensure authenticity and prevent spoofing.

## Beacon Identity

The `BeaconIdentity` struct is used for outgoing beacons:

```rust
pub struct BeaconIdentity {
    pub name: String,
    pub ssh_public_key: SshPublicKey,
    pub ip: String,
    pub ssh_port: u16,
    pub signing_key: SigningKey,  // Private key for signing
}
```

## Beacon Flow

### Announcing

1. The local device constructs a `BeaconIdentity` with its information
2. Every 2 s (hardcoded interval), it signs and broadcasts the beacon; the whole
   announce window lasts `wait_secs` (configurable, default 10 s)
3. Beacons are sent on all available network interfaces, each to that interface's
   broadcast address
4. Announcing and listening use the **same** UDP port (`udp_port`, default 4242):
   one channel — sender targets `<broadcast>:udp_port`, listener binds
   `0.0.0.0:udp_port`. All devices must share the port; it is never negotiated in
   the beacon (the beacon's `port` field is the SSH port)

### Listening

1. regroup binds `0.0.0.0:udp_port` for incoming beacons
2. For each received beacon:
   - Verify the signature using the embedded signing public key
   - Check the beacon age (filter out stale beacons)
   - Verify IP consistency (the beacon's IP must match the sender)
   - Check trusted keys (if the device is known)
3. Valid beacons are forwarded to the interactive loop via a channel

## Signature Verification

Beacons are verified in `discovery::beacon::verify_beacon()`:

1. **Signature check**: Verify the beacon payload against the signing public key
2. **Age check**: Beacons older than a threshold are discarded
3. **IP check**: The sender's IP must match the beacon's IP field

## Trusted Key Checks

For known devices (already in the configuration):

1. The beacon's signing public key must match the pinned key in `device.json`
2. If the signing key differs:
   - Prompt the user to trust the new key (expected on reinstall)
   - OR reject as potential spoofing/MITM
3. The user's decision is stored in the interactive state

## Local Identity

The `local.rs` module detects the local device's identity:

- **Hostname**: via `hostname` command or `std::env`
- **IP address**: detected from network interfaces
- **SSH server public key**: extracted from `/etc/ssh/ssh_host_*_key.pub` or via `ssh-keyscan`

Note: Termux on Android randomizes hostnames, so **SSH host key is the only reliable identifier**.

## Network Utilities

The `net.rs` module provides:

- `get_local_interfaces()`: Returns all local network interfaces with IPs
- `my_ip()`: Returns the primary local IP address
- Beacon transmission helpers

## Key Functions

### `announce_beacon()`

Broadcasts the local device's beacon:

```rust
pub fn announce_beacon(
    identity: &BeaconIdentity,
    port: u16,
    wait_secs: u64,
    interfaces: &[Interface],
    running: Option<&AtomicBool>,
)
```

### `listen_beacons_continuous()`

Listens for incoming beacons in a loop:

```rust
pub fn listen_beacons_continuous(
    port: u16,
    sender: Sender<BeaconDevice>,
    local_ssh_public_key: SshPublicKey,
    running: Arc<AtomicBool>,
)
```

### `verify_beacon()`

Verifies a single beacon:

```rust
pub fn verify_beacon(
    data: &[u8],
    local_key: &SshPublicKey,
) -> Result<BeaconDevice, BeaconError>
```

## Error Handling

Beacon-related errors:

- `BeaconError::InvalidSignature`: Signature verification failed
- `BeaconError::InvalidFormat`: Malformed beacon data
- `BeaconError::TooOld`: Beacon is too old
- `BeaconError::IpMismatch`: Sender IP doesn't match beacon IP

## Testing

Beacon signing, verification, and naming rules have unit tests in `beacon.rs` under `#[cfg(test)]`.
