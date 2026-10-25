# regroup

Find machines by their SSH host key fingerprint and update `~/.ssh/config`.

## New Architecture: Distributed Configuration

Configuration is now stored in a **separate git repository** where each device has its own file. This allows:

- **No conflicts**: Each device writes to its own file
- **Easy sync**: Use git to synchronize configurations across devices on a controlled network
- **Auto-registration**: Devices automatically create their own configuration if missing

## How It Works

1. Each device has its own JSON file in the `devices/` directory of the config repository
2. On first run, a device auto-creates its file with self-detected information (hostname, IP, fingerprint)
3. You sync the repository across all devices on your controlled network
4. Each device reads all device files to discover the network topology

## Quick Start

### 1. Create a config repository

```bash
# On a shared location (e.g., a server or USB drive)
git init regroup-config
cd regroup-config
# Create devices directory (optional - will be created automatically)
mkdir devices
# Initialize git with your settings
git config user.email "your@email.com"
git config user.name "Your Name"
git add .
git commit -m "Initial config"
```

### 2. On each device

```bash
# Build regroup
cd /path/to/regroup
git pull  # Make sure you have the latest code
cargo build --release

# Run regroup with your config repository
./target/release/regroup -c /path/to/regroup-config

# On first run, it will:
# 1. Clone the config repo if it doesn't exist locally
# 2. Auto-create a device file for this machine
# 3. Commit and you can push to sync with other devices
```

### 3. Sync configurations

After running on all devices, sync the repository:

```bash
cd /path/to/regroup-config
git pull  # Get latest changes
git push  # Push your device config
```

## Usage

```
regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config. Every run announces this machine over UDP broadcast while
listening for the other machines' announcements during the same window.

NEW: Configuration is now stored in a separate git repository where each
device has its own file. Devices auto-fill their information if missing.

Usage:
  ./regroup                announce + listen, then update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -c REPO        use REPO as config repository (default: ./regroup-config)
  ./regroup -r             refresh config from git repo before running
  ./regroup 192.168.43     also scan that /24 once (if broadcast is filtered)
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure:
  REPO/
    settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
    devices/
      <hostname>.json       (each device's config: name, fingerprint, user, port)

On first run, a new git repository is created with a devices/ directory.
Each device auto-creates its own config file with detected information.
Sync the repository across devices on a controlled network.

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
```

## Configuration Structure

### Repository Layout

```
regroup-config/
├── settings.json          # Global settings (optional)
└── devices/
    ├── ordi.json         # Device: ordi
    ├── nas.json          # Device: nas
    └── laptop.json       # Device: laptop
```

### Device Configuration File

Each device file (`devices/<name>.json`) contains:

```json
{
  "name": "ordi",
  "fingerprint": "SHA256:abcdef123456...",
  "user": "matthias",
  "port": 22
}
```

Fields:
- `name`: Device hostname (auto-detected from system hostname)
- `fingerprint`: SSH host key fingerprint (SHA256 or MD5 format)
- `user`: SSH username (defaults to current user)
- `port`: SSH port (defaults to 22)

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

## Workflow Examples

### Setting up a new network

```bash
# On a central server or laptop
mkdir ~/regroup-config
cd ~/regroup-config
git init

# On device 1 (ordi)
./regroup -c ~/regroup-config
# Auto-creates: ~/regroup-config/devices/ordi.json

# On device 2 (nas)
./regroup -c ~/regroup-config
# Auto-creates: ~/regroup-config/devices/nas.json

# Sync to a central location
cd ~/regroup-config
git add .
git commit -m "Add ordi and nas"
git remote add origin git@server:/path/to/regroup-config.git
git push -u origin main

# On other devices, clone and use
 git clone git@server:/path/to/regroup-config.git
./regroup -c ~/regroup-config
```

### Adding a new device

```bash
# On the new device (laptop)
./regroup -c ~/regroup-config
# Auto-creates ~/regroup-config/devices/laptop.json

# Commit and sync
cd ~/regroup-config
git add devices/laptop.json
git commit -m "Add laptop"
git push
```

### Using with USB drive for air-gapped networks

```bash
# On a machine with the USB drive mounted at /media/usb
./regroup -c /media/usb/regroup-config

# After running on all devices, sync manually
# Copy the updated files from the USB drive to other machines
```

### Getting the SSH fingerprint

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
- Git (optional, but recommended for config sync)

## Legacy Configuration

The old single-file `regroup.json` configuration is still supported for backward compatibility, but new installations should use the distributed configuration approach.

## License

MIT License
