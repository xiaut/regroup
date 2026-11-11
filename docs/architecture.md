# Architecture

## Big Picture

A **regroup** process does three things simultaneously:

1. **Announce** itself every `wait_secs` on all interfaces (signed UDP beacon: name, SSH host public key, signing key, user, port)
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
- **SSH host key** is the unique identifier for a machine (never hostname or IP, which can change)
- Each device has an Ed25519 **signing key** used to sign beacons
- Signing keys are pinned at first acceptance; different keys trigger a rotation prompt

#### Beacon Protocol
- UDP-based discovery on a configurable port (default: 9876)
- Beacons are signed to prevent spoofing
- Contains: device name, SSH public key, signing public key, user, port, IP
- Beacon age is checked to filter out stale announcements

#### Configuration Storage
- Central git repository at `~/.regroup-config/git/`
- Each device has its own directory: `devices/<name>/`
- Decentralized: every device can push/pull from every other device
- Last-writer-wins for conflict resolution
- **Write isolation**: Each device only writes its own `device.json`; accepting a device adds a git remote and pulls its authoritative data

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
├── git/
│   ├── .git/                # Git repository
│   ├── settings.json        # Global settings (udp_port, wait_secs)
│   └── devices/
│       └── <device-name>/
│           ├── device.json          # Device metadata (name, keys, user, port)
│           ├── users/
│           │   └── <user>.json      # Known users on this device
│           └── connections/
│               └── <id>.json      # Connection files
└── .gitignore
```

See [Storage Rules](storage.md) for detailed constraints.
