# regroup

Find machines by their SSH host key fingerprint and update `~/.ssh/config`.

## Configuration & Sync

**Configuration is stored in `~/.regroup-config/`** - a git repository where:
- Each device has its own JSON file in `devices/`
- Git remotes are **automatically configured** for all other devices
- Decentralized sync via `git pull --all` and `git push --all`
- One-way SSH connections are handled gracefully

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

### On second device:

```bash
./regroup
```

This will:
1. Create local `~/.regroup-config/`
2. Create its own device file
3. Automatically add the first device as a git remote
4. Push to the first device's config repo

### On first device (or any device):

```bash
# Pull changes from all remotes
./regroup -r

# This will pull from all known device remotes
# and update SSH config accordingly
```

## How It Works

### Automatic Git Remote Management

1. **On every run**, regroup:
   - Scans all device configs in `~/.regroup-config/devices/`
   - Adds git remotes for all other devices (except self)
   - Removes stale remotes (devices no longer in config)
   - Updates SSH config for each device

2. **Remote URLs** follow this format:
   ```
   user@hostname:~/.regroup-config
   ```

3. **Sync process**:
   - `-r` flag: Pulls from all remotes before discovery
   - After updating SSH config: Pushes to all remotes
   - One-way SSH is OK - failures are logged as warnings

### Example Workflow

```
Device A (hostname: ordi, user: alice)
  └── ~/.regroup-config/
      ├── devices/ordi.json
      └── settings.json

Device B runs regroup:
  1. Creates ~/.regroup-config/devices/laptop.json
  2. Adds remote: alice@ordi:~/.regroup-config
  3. Pushes to ordi's repo
  4. Updates SSH config to connect to ordi

Device A runs regroup -r:
  1. Pulls from laptop's remote (alice@ordi has laptop as remote)
  2. Gets laptop's device config
  3. Adds laptop as a remote
  4. Updates SSH config for laptop
```

## Usage

```
regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config.

Configuration is stored in ~/.regroup-config/ git repository where each
device has its own JSON file in the devices/ directory. Git remotes are
automatically configured for all other devices.

Usage:
  ./regroup                announce + listen, then update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -r             refresh: pull from all git remotes before running
  ./regroup 192.168.43     also scan that /24 once (if broadcast is filtered)
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure (~/.regroup-config/):
  settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
  devices/
    <hostname>.json       (each device's config: name, fingerprint, user, port)

Git remotes (automatically managed):
  user@hostname:~/.regroup-config  (for each other device)

Sync behavior:
  - git pull --all on startup (with -r flag)
  - git push --all after updating SSH config
  - One-way SSH connections handled gracefully (warnings only)

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
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

**Device 1 (ordi):**
```bash
# First run - creates config and initializes git
./regroup

# Config is now at ~/.regroup-config/ with ordi.json
```

**Device 2 (laptop):**
```bash
# First run - creates config, adds ordi as remote, pushes to ordi
./regroup

# Now ordi has laptop as a remote, and laptop has ordi as a remote
```

**Sync on Device 1:**
```bash
# Pull laptop's config
./regroup -r
```

### Manual sync (if automatic push fails)

```bash
# On any device
cd ~/.regroup-config
git pull --all
git push --all
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

# Commit and sync
git add .
git commit -m "Add newdevice"
git push --all

# On other devices, run with -r to pull the new config
./regroup -r
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

### SSH connection issues
Make sure:
1. SSH keys are set up between devices
2. Hostnames are resolvable (use IP addresses if DNS doesn't work)
3. SSH config is correct (regroup updates this automatically)

### Git push/pull failures
These are expected if:
- A device is offline
- SSH is one-way only
- Firewall blocks connections

Regroup handles these gracefully with warnings. The sync will work through other paths.

## License

MIT License
