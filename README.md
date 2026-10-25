# regroup

Find machines by their SSH host key fingerprint and update `~/.ssh/config`.

## Configuration & Sync

**Configuration is stored in `~/.regroup-config/`** - a git repository where:
- Each device has its own JSON file in `devices/`
- Git remotes are **automatically configured** for all other devices (using hostnames)
- Discovery and sync run in a **loop** until no new devices are found
- Each device **owns** its own config file and can overwrite only it
- If local device config differs from remote, user is **prompted** to resolve

## Quick Start

### First run on any device:

```bash
./regroup
```

This will:
1. Create `~/.regroup-config/` directory
2. Initialize a git repository
3. Create `settings.json` with defaults
4. Create `devices/<hostname>.json` with auto-detected info
5. Commit everything to git
6. Enter discovery loop until no new devices are found

### On second device:

```bash
./regroup
```

This will:
1. Create local `~/.regroup-config/`
2. Create its own device file
3. Automatically add the first device as a git remote (using hostname)
4. Push to the first device's config repo
5. Enter discovery loop

## How It Works

### Discovery and Sync Loop

On every run, regroup performs:

```
Loop:
  1. Refresh git remotes (add/remove based on device list)
  2. If -r flag or iteration > 1: git pull --all
  3. UDP beacon announcement + listening
  4. Fallback TCP subnet scan (if needed)
  5. Fingerprint collection
  6. SSH config updates for discovered machines
  7. git push --all (after SSH config update)
  8. Check if new devices were found
  9. If yes, repeat from step 1
  10. If no, exit loop
```

This loop handles:
- **IP address changes**: Devices are identified by hostname, not IP
- **Network discovery**: Multiple iterations find all devices
- **Git sync**: Changes propagate through the network
- **One-way SSH**: Failures are logged as warnings, not errors

### Automatic Git Remote Management

1. **On every iteration**:
   - Remotes are added for all devices (except self)
   - Stale remotes (devices no longer in config) are removed
   - Remote URLs use format: `user@hostname:~/.regroup-config`

2. **Sync operations**:
   - `git pull --all`: Pulls from all remotes
   - `git push --all`: Pushes to all remotes
   - Failures are logged as warnings (one-way SSH is OK)

### Device Ownership

- Each device owns its own config file (`devices/<hostname>.json`)
- A device can only overwrite its own file
- If remote config differs from local, user is prompted to resolve:
  - Keep local version
  - Use remote version
  - Abort

## Usage

```
regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config.

Configuration is stored in ~/.regroup-config/ git repository where each
device has its own JSON file in the devices/ directory. Git remotes are
automatically configured for all other devices (using hostnames, not IPs).

Discovery and sync run in a loop until no new devices are found.

Usage:
  ./regroup                announce + listen + sync loop, update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -r             pull from git remotes before starting sync loop
  ./regroup 192.168.43     also scan that /24 once
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure (~/.regroup-config/):
  settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
  devices/
    <hostname>.json       (each device's config: name, fingerprint, user, port)

Git remotes (automatically managed using hostnames):
  user@hostname:~/.regroup-config  (for each other device)

Sync behavior:
  - Loop until no new devices are discovered
  - git pull --all on each iteration (except first, or with -r)
  - git push --all after updating SSH config
  - One-way SSH connections handled gracefully
  - User prompted to resolve own config conflicts

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
  3  user declined conflict resolution
```

## Configuration Structure

### Device Configuration File

Each device file (`devices/<name>.json`) contains:

```json
{
  "name": "hostname",
  "fingerprint": "SHA256:abcdef123456...",
  "user": "username",
  "port": 22
}
```

Fields:
- `name`: Device hostname (auto-detected) - **required**
- `fingerprint`: SSH host key fingerprint (SHA256 or MD5 format) - **required**
- `user`: SSH username (defaults to current user, omitted if empty)
- `port`: SSH port (defaults to 22, omitted if default)

**Important**: Devices are identified by **hostname**, not IP address. This allows the system to handle IP changes automatically.

### Global Settings

Optional `settings.json` file:

```json
{
  "udp_port": 4242,
  "wait_secs": 10,
  "batch_size": 64,
  "subnets": ["192.168.43", "192.168.1"]
}
```

Fields:
- `udp_port`: UDP port for beacon announcements (default: 4242)
- `wait_secs`: Time to wait for beacons (default: 10 seconds)
- `batch_size`: Parallel connections for subnet scanning (default: 64)
- `subnets`: Array of /24 subnet prefixes to scan as fallback

## Practical Examples

### Two devices on a local network

**Device A (hostname: ordi):**
```bash
# First run - creates config, initializes git, enters discovery loop
./regroup

# Config is now at ~/.regroup-config/ with ordi.json
# Git remote for laptop will be added when laptop is discovered
```

**Device B (hostname: laptop):**
```bash
# First run - creates config, enters discovery loop
./regroup

# Will discover ordi via UDP beacons
# Will add ordi as git remote: ordiuser@ordi:~/.regroup-config
# Will push its config to ordi
```

**Device A:**
```bash
# On next run or when laptop is discovered:
# Will add laptop as git remote: laptopuser@laptop:~/.regroup-config
# Will pull laptop's config
# Both devices now have each other's configs
```

### With -r flag (force refresh)

```bash
# On any device, force pull from all remotes before discovery
./regroup -r

# Useful when you know there are updates but UDP discovery might miss them
```

### Adding a device manually

```bash
# On any device, add a new device config
cat > ~/.regroup-config/devices/newdevice.json << EOF
{
  "name": "newdevice",
  "fingerprint": "SHA256:...",
  "user": "username"
}
EOF

# Commit and the loop will handle sync
git add .
git commit -m "Add newdevice"

# On next run, newdevice will be added as remote and synced
./regroup
```

## Getting SSH Fingerprints

To find a machine's fingerprint:

**Ubuntu/Debian:**
```bash
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

**Termux:**
```bash
ssh-keygen -lf $PREFIX/etc/ssh/ssh_host_ed25519_key.pub
```

## Building

### Termux (Android)

```bash
pkg install rust openssh git
cargo build --release
```

### Ubuntu/Debian

```bash
sudo apt install cargo openssh-client git
cargo build --release
```

## Requirements

- Rust (cargo)
- OpenSSH client (`ssh-keyscan`, `ssh-keygen`)
- Git
- SSH access between devices (one-way is OK)

## Troubleshooting

### "git remote errors (one-way SSH is OK)"
This is normal! It means some devices can't push/pull to each other. As long as:
- Each device can reach at least one other device
- The network forms a connected graph
Then all configs will eventually sync.

### "[sync] ssh: Could not resolve hostname..."
This means DNS can't resolve the hostname. Solutions:
1. Use hostnames that are in /etc/hosts or DNS
2. Add entries to /etc/hosts for all devices
3. Use the IP address as hostname (temporary solution)

### SSH connection issues
Make sure:
1. SSH keys are set up between devices
2. Hostnames are resolvable
3. SSH config is correct (regroup updates this automatically)

### Git push/pull failures
These are expected if:
- A device is offline
- SSH is one-way only
- Firewall blocks connections

Regroup handles these gracefully with warnings. The sync will work through other paths.

### Conflict resolution
When your device's config differs from the remote, you'll be prompted:
```
CONFLICT: Device config for 'yourhost' differs from remote
Local version:
{...}

Remote version:
{...}

Options:
  1. Keep local version
  2. Use remote version
  3. Abort
Choice [1/2/3]:
```

## License

MIT License
