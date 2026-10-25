# regroup

Find machines by their SSH host key fingerprint and update `~/.ssh/config`.

## Configuration

**Configuration is stored in `~/.regroup-config/`** - a git repository where each device has its own JSON file in the `devices/` directory.

Each device automatically creates its own configuration file with self-detected information (hostname, SSH fingerprint, user) on first run.

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

### On other devices:

```bash
# Clone the config from first device
git clone ~/regroup-config ~/.regroup-config
# Or copy the directory directly
cp -r /path/to/regroup-config ~/.regroup-config

# Then run regroup
./regroup
```

### Sync changes:

```bash
# Pull latest changes before running
./regroup -r

# Or manually in the config directory
cd ~/.regroup-config
git pull
```

## Usage

```
regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config.

Configuration is stored in ~/.regroup-config/ git repository where each
device has its own JSON file in the devices/ directory. Devices auto-fill
their information if missing.

Usage:
  ./regroup                announce + listen, then update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -r             refresh config from git repo before running
  ./regroup 192.168.43     also scan that /24 once (if broadcast is filtered)
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure (~/.regroup-config/):
  settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
  devices/
    <hostname>.json       (each device's config: name, fingerprint, user, port)

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
```

## Configuration Structure

### Repository Layout

```
~/.regroup-config/
├── settings.json          # Global settings (optional)
└── devices/
    ├── ubuntu-MacBookPro.json
    ├── ordi.json
    └── nas.json
```

### Device Configuration File

Each device file (`devices/<name>.json`) contains:

```json
{
  "name": "ubuntu-MacBookPro",
  "fingerprint": "SHA256:abcdef123456...",
  "user": "matthias",
  "port": 22
}
```

Fields:
- `name`: Device hostname (auto-detected)
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

## How It Works

1. **First run**: Creates `~/.regroup-config/` with git repo, settings, and your device file
2. **Subsequent runs**: Loads all device configs from `~/.regroup-config/devices/`
3. **Auto-registration**: If your device's config is missing, it's auto-created
4. **Sync**: Use `./regroup -r` or manually `git pull` in `~/.regroup-config/`

## Workflow Examples

### Setting up a new network

```bash
# On first device
./regroup
# This creates ~/.regroup-config/ with your device config

# Commit and push to a shared location
cd ~/.regroup-config
git remote add origin git@server:/path/to/regroup-config.git
git push -u origin master

# On second device
git clone git@server:/path/to/regroup-config.git ~/.regroup-config
./regroup
# This creates a device file for the second machine

# On second device, push changes
cd ~/.regroup-config
git add .
git commit -m "Add second device"
git push

# On first device, pull changes
./regroup -r
```

### Using with USB drive for air-gapped networks

```bash
# On first device
./regroup
cp -r ~/.regroup-config /media/usb/

# On second device (after copying from USB)
cp -r /media/usb/regroup-config ~/.regroup-config
./regroup
cp -r ~/.regroup-config /media/usb/
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
- Git (optional, but recommended for config sync)

## License

MIT License
