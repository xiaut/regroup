# regroup

Find devices by their SSH host key and update `~/.ssh/config`.

## Configuration & Sync

**Configuration is stored in `~/.regroup-config/git/`** - a git repository where:
- Each device owns a directory `devices/<name>/` holding its latest known
  state (`device.json`), its known users (`users/<user>.json`) and its
  connections (`connections/`)
- Devices are identified by their **SSH public key** (hostnames and IPs can change)
- Devices announce themselves via **signed UDP beacons** (Ed25519) carrying
  device data only — name, keys, IP, port; regroup listens continuously and
  prompts for each new device. No user data is ever announced on the network
- Git remotes are **automatically configured** for all other devices (using hostnames)
- Devices only ever write inside their own directory (`users/` and
  `connections/`); `device.json` can also be written by a device that
  discovers new knowledge about a peer, and git sync resolves diverging
  writes with the **most recent commit** (last-writer-wins)
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
3. Create `devices/<hostname>/device.json` with auto-detected info
4. Enter continuous interactive mode: announce + listen for beacons

### On second device:

```bash
./regroup
```

This will:
1. Create local `~/.regroup-config/git/`
2. Create its own device directory (auto-detected info)
3. Discover the first device via its UDP beacon and prompt to add it
4. Enter continuous interactive mode (press `s` to pull from and push to git remotes)

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
1. Announce this device via signed UDP beacons (every ~2 s, one per interface)
2. Listen for other devices' beacons continuously
   - Each beacon's Ed25519 signature is verified; malformed or forged beacons are rejected
   - Beacons from an accepted device must be signed with the signing key
     pinned in its device file; a different key means reinstall (you are asked)
     or spoofing (the beacon is rejected)
   - Beacons older than 5 minutes, or dated too far in the future, are rejected
   - Beacons announcing an SSH public key we already know only update the
     device's IP in ~/.ssh/config (silently, when the IP changed)
3. New devices appear in the "Detected" panel of the interactive ui; select
   one with Up/Down and add it with Enter/y ('n' ignores it for this run);
   in plain (piped) mode, each one is prompted (y/N) instead
4. Accepted devices are verified against the live server (see below), then
   written to ~/.ssh/config, ~/.ssh/known_hosts and the git config repository
   (committed)
5. Pressing 's' pulls from and pushes to all configured git remotes
```

The interactive ui shows two device panels side by side — **Detected** on
the left (announced, waiting for a decision), **Added** on the right (in the
configuration) — above an Logs panel and the current question. General
messages (git syncs, dry run) go to the Logs panel; messages about one
device (IP updates, signing-key changes, security warnings) appear as a
brief animated status on that device's row instead of cluttering the log.
Security warnings are additionally kept in the Logs panel so they cannot
be missed.

This handles:
- **IP address changes**: devices are identified by SSH public key, not IP
- **Spoofing / MITM**: two independent checks, at two different layers:
  - *the server*: before anything is written, the host key announced in a
    beacon is verified against the SSH server at the announced IP
    (`ssh-keyscan`); a mismatch (forged beacon or man-in-the-middle) aborts
    the add, or — for a known device whose IP changed — keeps the last
    verified IP and warns loudly
  - *the announcer*: each device's Ed25519 signing key is pinned in its
    device file at first acceptance; beacons signed with any other key are
    detected instantly (no network access needed) and the user is asked to
    confirm — expected after a reinstall, suspicious otherwise
- **Git sync**: changes propagate through the network

### Git Remote Management

1. **Remote URLs** use the format: `user@hostname:~/.regroup-config/git`

   Beacons carry no user data (users are configuration, not discovery), so the
   SSH user of a device comes from its configuration: the first user it
   declared in `users/` (arrived through git sync), or a **temporary user**
   you add from the Users panel of the interactive ui (in memory only, just
   enough to build the remote URL and run the first sync). Devices with no
   known user are skipped until one is known. Remotes pointing at a regroup
   config repository are removed when the corresponding device disappears
   from the config; other remotes (e.g. a backup repo you added yourself) are
   left untouched.

2. **Sync operations** (triggered with 's' in interactive mode):
   - `git pull` from all remotes
   - `git push` to all remotes
   - Failures are logged as warnings (one-way SSH is OK)

3. **Repository configuration** (set automatically on init):
   - `receive.denyCurrentBranch = updateInstead`: Allows pushing to checked-out branches
   - `pull.rebase = true`: Uses rebase instead of merge by default
   
   **Note**: With `pull.rebase = true`, manual `git pull` commands will use rebase automatically.

### Device Ownership & Name Conflicts

- Each device owns its directory (`devices/<name>/`): its `users/` and
  `connections/` files are only ever written by the device itself, and a
  user running regroup may only modify connection files where their own name
  is the member located on this device
- When you accept a new device whose name is already used by another device,
  regroup asks you to choose a new name. A name is considered taken when it
  matches:
  - a device in the regroup config (`devices/<name>/`), or
  - an existing `Host` block in `~/.ssh/config` (regroup-managed or your own)
- The first free name (`laptop-2`, `-3`, ...) is suggested as the default;
  press Enter to accept it, or type any other name. Names that are also taken
  are rejected and you are asked again. Existing entries are never overwritten

## Usage

```
regroup — find devices by their SSH host key and update ~/.ssh/config.
Devices announce themselves via signed UDP beacons; regroup listens for them
continuously and prompts for each new device.

Usage:
  ./regroup                continuous interactive mode, update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup remove <name>  remove a device (device dir, ~/.ssh/config block, git remote)
  ./regroup wipe [--yes]   delete the entire regroup configuration
  ./regroup selftest       built-in checks (external tools, ssh-keygen fingerprint)
  ./regroup setup-cd       install 'regroup-cd' shell function to change to config directory
  ./regroup -h             this help

Interactive commands (full-screen mode):
  Up/Dn           - select a device in the Detected panel
  Enter/y         - add the selected device (it moves to the Added panel)
  n               - ignore the selected device (its beacons are ignored this run)
  s               - sync git (pull from remotes, push to remotes)
  d               - delete a known device (select in the Added panel, Enter confirms, Esc cancels)
  w               - wipe the whole configuration and quit (Enter confirms, Esc cancels)
  c               - install the 'regroup-cd' shell function (Enter confirms, Esc cancels)
  PgUp/PgDn       - scroll the Logs panel
  q               - quit the program

Config repository structure (~/.regroup-config/git/):
  settings.json                     (optional: udp_port, wait_secs)
  devices/
    <hostname>/                     (one directory per device)
      device.json                  (latest known state: name, ssh_public_key, port)
      users/<user>.json             (known users of the device, written by the device itself)
      connections/<conn>.json       (connections of the device, owner-scoped)

Exit codes:
  0  normal exit
  1  selftest failure
  2  configuration or environment error
```

## Configuration Structure

### Device Directory

Each device owns one directory `devices/<name>/`. `device.json` contains
the latest known state:

```json
{
  "name": "hostname",
  "ssh_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...",
  "signing_public_key": "MCowBQYDK2VwAyEA...",
  "port": 22
}
```

Fields:
- `name`: Device hostname (auto-detected) - **required**
- `ssh_public_key`: SSH server host public key (OpenSSH format) - **required**; this is the unique identifier of a device (hostnames and IPs can change)
- `signing_public_key`: Ed25519 signing public key (base64, **required**) — written by regroup when the device file is created; beacons signed with any other key trigger a confirmation prompt (reinstall or spoofing)
- `port`: SSH port (defaults to 22; devices discovered via beacon keep their announced port, e.g. Termux's 8022)

There is no `user` field: users live in `users/<user>.json`, written by the
device itself (one file per user that runs regroup there).

**Important**: Devices are identified by their **SSH public key**, not by hostname or IP address. This allows the system to handle host name and IP changes automatically.

On conflicts (the same file written by several devices or by several users
of one device), the most recent commit wins.

### Known Users (`users/`)

`devices/<name>/users/<user>.json` holds one file per user known to run on
the device. Only the device itself writes there — the file is created the
first time a user runs regroup, then propagates to other devices through
git sync:

```json
{
  "name": "username",
  "first_seen": "2026-11-05T10:00:00Z"
}
```

### Connections (`connections/`) — not enabled yet

Connection files (`devices/<name>/connections/`) are prepared but the
procedures to create and remove connections (dedicated SSH key pair,
installation of the public key on the target device) are not implemented
yet. The storage layer is in place: the member of the connection located on
the device whose directory the file lives in is written as a bare user
name, the other one as `user@device`, and the arrow keeps the real
direction:

- outgoing: `<user>-><user>@<device>.json` (e.g. `alice->bob@laptop.json`)
- incoming: `<user>@<device>-><user>.json` (e.g. `alice@ordi->bob.json`)

A user may only modify (create, edit, delete) connection files in which
their own name is the member on the current device.

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
# On next run, laptop's beacon is received (or press 's' after adding
# laptop as a remote manually)
# Both devices now have each other's configs
```

### Syncing with git remotes

```bash
# In interactive mode, press s to pull from and push to all remotes
./regroup
# ... then press s

# Useful when you know there are updates but UDP discovery might miss them
```

### Removing devices

```bash
# Interactive mode: press d, select a device with arrow keys, Enter confirms, Esc cancels
./regroup

# From the command line
./regroup remove <name>

# Delete the whole configuration (device dirs, git repo, ~/.ssh/config blocks)
./regroup wipe --yes
```

Removing a device deletes its device directory (committed, so the removal
propagates to other devices on the next git sync), its `~/.ssh/config`
block, its git remote and the `~/.ssh/known_hosts` entries of its last
known IP. Devices that were removed on another device get the same
local cleanup automatically when they sync. A removed device that is
still online will be proposed again as a new device: removing means
forgetting, not banning.

`regroup wipe` does not touch `~/.ssh/known_hosts` (past IPs are not
recorded) and is handy for testing a fresh setup.

### Adding a device manually

```bash
# On any device, add a new device config
# signing_public_key is required: take it from the device's own config
# (~/.regroup-config/git/devices/<name>/device.json on that device)
mkdir -p ~/.regroup-config/git/devices/newdevice
cat > ~/.regroup-config/git/devices/newdevice/device.json << EOF
{
  "name": "newdevice",
  "ssh_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...",
  "signing_public_key": "MCowBQYDK2VwAyEA..."
}
EOF

# Commit and regroup will use it on the next run
cd ~/.regroup-config/git
git add devices/newdevice/device.json
git commit -m "Add newdevice"

# On next run, newdevice will be recognized by its public key and synced
./regroup
```

## Reading SSH Host Public Keys

To find a device's host public key:

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
Two devices may share a hostname (e.g. two devices with the same default
name). When you accept a device whose name is already taken, regroup asks
you to pick another name:

```
  ! Name 'laptop' is already used by another device or an existing ~/.ssh/config entry.
  New name for this device (empty = laptop-2):
```

Press Enter to accept the suggested free name, or type your own. If your
choice is also taken you will be asked again. Both devices keep working
under their own names.

## License

MIT License
