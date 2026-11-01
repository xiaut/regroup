# regroup

Find machines by their SSH host key and update `~/.ssh/config`.

## Configuration & Sync

**Configuration is stored in `~/.regroup-config/git/`** - a git repository where:
- Each device has its own JSON file in `devices/`
- Devices are identified by their **SSH public key** (hostnames and IPs can change)
- Devices announce themselves via **signed UDP beacons** (Ed25519); regroup
  listens continuously and prompts for each new device
- Git remotes are **automatically configured** for all other devices (using hostnames)
- Each device **owns** its own config file and can overwrite only it
- If a new device's **name is already taken**, you are **asked to choose a new name**

## Quick Start

### First run on any device:

```bash
./regroup
```

This will:
1. Create `~/.regroup-config/git/` directory and initialize a git repository with automatic configuration:
   - `receive.denyCurrentBranch` set to `updateInstead` (allows pushing to checked-out branches)
   - `pull.rebase` set to `true` (maintains linear history)
2. Create `settings.json` with defaults
3. Create `devices/<hostname>.json` with auto-detected info
4. Enter continuous interactive mode: announce + listen for beacons

### On second device:

```bash
./regroup
```

This will:
1. Create local `~/.regroup-config/git/`
2. Create its own device file
3. Discover the first device via its UDP beacon and prompt to add it
4. Enter continuous interactive mode (press `r` to pull from and push to git remotes)

## First Manual Git Sync

When setting up your first device or adding devices to an existing network, you may need to perform a manual git sync:

### From the device itself (if you have SSH access to other devices):

```bash
cd ~/.regroup-config/git
git remote add <other-device> user@other-device:~/.regroup-config/git
git push <other-device> main
```

This pushes your config to the other device.

### From another device (if you're setting up a new device):

```bash
cd ~/.regroup-config/git
git remote add <new-device> user@new-device:~/.regroup-config/git
git pull <new-device> main
```

This pulls the new device's config to your existing setup.

**Authentication direction matters:**
- If you can SSH from A → B, you can push from A to B
- If you can SSH from B → A, you can pull from A on B
- One-way authentication is sufficient for sync to work

## How It Works

### Continuous Discovery

On every run, regroup enters continuous interactive mode:

```
1. Announce this machine via signed UDP beacons (every ~2 s, one per interface)
2. Listen for other machines' beacons continuously
   - Each beacon's Ed25519 signature is verified; unsigned/forged beacons are rejected
   - Beacons older than 5 minutes are rejected
   - Beacons announcing an SSH public key we already know only update the
     machine's IP in ~/.ssh/config (silently, when the IP changed)
3. New machines are queued and the user is prompted (y/N) for each one
4. Accepted machines are written to ~/.ssh/config, ~/.ssh/known_hosts and
   the git config repository (committed)
5. Pressing 'r' pulls from and pushes to all configured git remotes
```

This handles:
- **IP address changes**: devices are identified by SSH public key, not IP
- **Spoofing**: beacons are Ed25519-signed and age-checked
- **Git sync**: changes propagate through the network

### Git Remote Management

1. **Remote URLs** use the format: `user@hostname:~/.regroup-config/git`

   Devices without a `user` field (e.g. added via beacon) fall back to the
   current local user for the remote URL; set `"user"` in the device file to
   override. Remotes pointing at a regroup config repository are removed when
   the corresponding device disappears from the config; other remotes (e.g.
   a backup repo you added yourself) are left untouched.

2. **Sync operations** (triggered with 'r' in interactive mode):
   - `git pull` from all remotes
   - `git push` to all remotes
   - Failures are logged as warnings (one-way SSH is OK)

3. **Repository configuration** (set automatically on init):
   - `receive.denyCurrentBranch = updateInstead`: Allows pushing to checked-out branches
   - `pull.rebase = true`: Uses rebase instead of merge by default
   
   **Note**: With `pull.rebase = true`, manual `git pull` commands will use rebase automatically.

### Device Ownership & Name Conflicts

- Each device owns its own config file (`devices/<name>.json`)
- When you accept a new device whose name is already used by another machine,
  regroup asks you to choose a new name. A name is considered taken when it
  matches:
  - a machine in the regroup config (`devices/` directory), or
  - an existing `Host` block in `~/.ssh/config` (regroup-managed or your own)
- The first free name (`laptop-2`, `-3`, ...) is suggested as the default;
  press Enter to accept it, or type any other name. Names that are also taken
  are rejected and you are asked again. Existing entries are never overwritten

## Usage

```
regroup — find machines by their SSH host key and update ~/.ssh/config.
Devices announce themselves via signed UDP beacons; regroup listens for them
continuously and prompts for each new device.

Usage:
  ./regroup                continuous interactive mode, update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup selftest       built-in checks (tools, config parsing)
  ./regroup setup-cd       install 'regroup-cd' shell function to change to config directory
  ./regroup -h             this help

Interactive commands:
  y/N             - add or skip the current device
  r               - refresh git (pull from remotes, push to remotes)
  q               - quit the program

Config repository structure (~/.regroup-config/git/):
  settings.json           (optional: udp_port, wait_secs)
  devices/
    <hostname>.json       (each device's config: name, ssh_public_key, user, port)

Exit codes:
  0  normal exit
  1  selftest failure
  2  configuration or environment error
```

## Configuration Structure

### Device Configuration File

Each device file (`devices/<name>.json`) contains:

```json
{
  "name": "hostname",
  "ssh_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...",
  "user": "username",
  "port": 22
}
```

Fields:
- `name`: Device hostname (auto-detected) - **required**
- `ssh_public_key`: SSH server host public key (OpenSSH format) - **required**; this is the unique identifier of a machine (hostnames and IPs can change)
- `user`: SSH username (defaults to current user for auto-created devices, omitted if empty)
- `port`: SSH port (defaults to 22; devices discovered via beacon keep their announced port, e.g. Termux's 8022)

**Important**: Devices are identified by their **SSH public key**, not by hostname or IP address. This allows the system to handle host name and IP changes automatically.

### Global Settings

Optional `settings.json` file:

```json
{
  "udp_port": 4242,
  "wait_secs": 10
}
```

Fields:
- `udp_port`: UDP port for beacon announcements (default: 4242)
- `wait_secs`: Duration of each announcement window (default: 10 seconds)

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
# First run - creates config, initializes git, enters interactive mode
./regroup

# Will discover ordi via its UDP beacon and prompt to add it
# ordi's entry is created in ~/.ssh/config and the config repo
```

**Device A:**
```bash
# On next run, laptop's beacon is received (or press 'r' after adding
# laptop as a remote manually)
# Both devices now have each other's configs
```

### Refreshing from git remotes

```bash
# In interactive mode, press r to pull from and push to all remotes
./regroup
# ... then press r

# Useful when you know there are updates but UDP discovery might miss them
```

### Adding a device manually

```bash
# On any device, add a new device config
cat > ~/.regroup-config/git/devices/newdevice.json << EOF
{
  "name": "newdevice",
  "ssh_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...",
  "user": "username"
}
EOF

# Commit and regroup will use it on the next run
cd ~/.regroup-config/git
git add devices/newdevice.json
git commit -m "Add newdevice"

# On next run, newdevice will be recognized by its public key and synced
./regroup
```

## Reading SSH Host Public Keys

To find a machine's host public key:

**Ubuntu/Debian:**
```bash
cat /etc/ssh/ssh_host_ed25519_key.pub
```

**Termux:**
```bash
cat $PREFIX/etc/ssh/ssh_host_ed25519_key.pub
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

### Name conflicts
Two machines may share a hostname (e.g. two devices with the same default
name). When you accept a device whose name is already taken, regroup asks
you to pick another name:

```
  ! Name 'laptop' is already used by another machine or an existing ~/.ssh/config entry.
  New name for this device (empty = laptop-2):
```

Press Enter to accept the suggested free name, or type your own. If your
choice is also taken you will be asked again. Both machines keep working
under their own names.

## License

MIT License
