# Configuration Architecture Update

## Summary

**Configuration is now always stored in `~/.regroup-config/`** - a git repository where each device has its own JSON file in the `devices/` directory.

## Key Features

### 1. Hostname-based Identification

- **Devices are identified by hostname, not IP address**
- This allows the system to handle IP address changes automatically
- Git remotes use hostname: `user@hostname:~/.regroup-config`

### 2. Discovery and Sync Loop

The main algorithm now runs in a loop:

```
Loop (max 10 iterations):
  1. Refresh git remotes (add/remove based on current device list)
  2. If -r flag OR iteration > 1: git pull --all
  3. UDP beacon announcement + listening
  4. Fallback TCP subnet scan (if needed)
  5. Fingerprint collection and matching
  6. SSH config updates for discovered machines
  7. git push --all (after SSH config update)
  8. Check if new devices were found
  9. If yes: continue loop
  10. If no: exit loop
```

This loop ensures:
- All devices are discovered even if they don't all respond in the first iteration
- Config changes propagate through the network
- IP address changes are handled automatically

### 3. Automatic Git Remote Management

- **Automatic addition**: When a new device is detected in the config, a git remote is automatically added
- **Automatic removal**: When a device is removed from config, its remote is automatically removed
- **Remote URL format**: `user@hostname:~/.regroup-config` (using hostname, not IP)
- **Self-exclusion**: A device never adds itself as a remote

### 4. Decentralized Sync

- **`git pull --all`**: Used on each iteration (except first) to pull from all remotes
- **`git push --all`**: Used after updating SSH config to push to all remotes
- **Graceful failures**: One-way SSH connections are handled with warnings only
- **With `-r` flag**: Pull from all remotes even on first iteration

### 5. Device Ownership

- Each device owns its own config file (`devices/<hostname>.json`)
- A device can only overwrite its own file
- **Conflict detection**: If local device config differs from what was pulled from remote:
  - User is prompted to resolve
  - Options: Keep local, Use remote, Abort
  - Exit code 3 if user aborts

## Configuration Structure

```
~/.regroup-config/
├── .git/                      # Git repository
├── settings.json             # Global settings (optional)
└── devices/
    ├── ubuntu-MacBookPro.json # Device 1 (this device)
    ├── ordi.json             # Device 2 (remote added)
    └── nas.json              # Device 3 (remote added)
```

### Device File Format

```json
{
  "name": "hostname",           # Required - used as identifier
  "fingerprint": "SHA256:...",  # Required - SSH host key
  "user": "username",           # Optional (default: current user)
  "port": 22                    # Optional (default: 22)
}
```

**Important**: The `name` field must match the device's hostname. This is how devices are identified and how remotes are named.

### Global Settings Format

```json
{
  "udp_port": 4242,             # Optional (default: 4242)
  "wait_secs": 10,             # Optional (default: 10)
  "batch_size": 64,            # Optional (default: 64)
  "subnets": ["192.168.43"]   # Optional (default: [])
}
```

## Breaking Changes

1. **Configuration location is fixed**: Always `~/.regroup-config/` - cannot be changed
2. **Removed `-c` flag**: Config location is no longer configurable
3. **Old `regroup.json` format removed**: Completely replaced with per-device files
4. **New exit code 3**: User declined conflict resolution

## Migration Guide

### For new users:

Just run:
```bash
./regroup
```

Everything is set up automatically.

### For existing users with old `regroup.json`:

You must migrate manually:

1. **Backup your old config:**
   ```bash
   cp regroup.json regroup.json.bak
   ```

2. **Run regroup to create new structure:**
   ```bash
   ./regroup
   ```
   This creates `~/.regroup-config/` with your current device.

3. **Add other devices:**
   For each machine in your old `regroup.json`, create a file:
   
   ```bash
   cat > ~/.regroup-config/devices/ordi.json << EOF
   {
     "name": "ordi",
     "fingerprint": "SHA256:abc...",
     "user": "matthias"
   }
   EOF
   ```
   
   **Important**: The `name` must match the device's actual hostname.

4. **Copy global settings:**
   ```bash
   cat > ~/.regroup-config/settings.json << EOF
   {
     "udp_port": 4242,
     "wait_secs": 10,
     "batch_size": 64,
     "subnets": ["192.168.43"]
   }
   EOF
   ```

5. **Commit and sync:**
   ```bash
   cd ~/.regroup-config
   git add .
   git commit -m "Migrated from old config"
   git push --all 2>&1 | grep -v "one-way SSH is OK"  # Filter expected warnings
   ```

## Usage Examples

### Simple two-device network

**Device A (hostname: ordi):**
```bash
./regroup
# Creates config, initializes git, enters discovery loop
# Will add laptop as remote when discovered
```

**Device B (hostname: laptop):**
```bash
./regroup
# Creates config, enters discovery loop
# Discovers ordi via UDP beacons
# Adds ordi as git remote: ordiuser@ordi:~/.regroup-config
# Pushes its config to ordi
```

**Device A:**
```bash
# On next run or when laptop is discovered:
# Adds laptop as git remote: laptopuser@laptop:~/.regroup-config
# Pulls laptop's config
# Both devices now have each other's configs
```

### Manual sync when automatic fails

```bash
# On any device
cd ~/.regroup-config
git pull --all
git push --all
```

### Forcing refresh with -r

```bash
# On any device, force pull from all remotes before discovery
./regroup -r

# Useful when you know there are updates but UDP discovery might miss them
```

### Handling IP address changes

If a device's IP changes:
1. The device still announces with its hostname
2. Git remote is still valid (uses hostname)
3. SSH config gets updated with new IP
4. All other devices pick up the new IP automatically

## Technical Details

### Why Hostnames?

The key insight is that **IP addresses can change, but hostnames are stable**. By:
- Identifying devices by hostname
- Using hostnames in git remote URLs
- Using hostnames in SSH config

The system automatically handles IP address changes without any manual intervention.

### Git Remote Management

The `refresh_git_remotes()` function runs on every iteration:
1. Gets list of existing remotes
2. Removes remotes that don't correspond to any machine
3. Removes self as remote (never add self)
4. Adds remotes for all machines (except self)
5. Updates existing remotes if URL changed

### Sync Loop Logic

The loop continues until:
- No new devices are discovered in an iteration, OR
- Maximum iterations (10) is reached

On each iteration:
- Remotes are refreshed to match current device list
- Git pull from all remotes (if not first iteration or -r flag)
- Discovery via UDP beacons and TCP scan
- SSH config updates
- Git push to all remotes

### Conflict Detection

When updating SSH config for the local device:
1. Read current device file content
2. Compare with what we would write
3. If different, prompt user to resolve
4. User choices:
   - Keep local: Do nothing (current file is kept)
   - Use remote: Overwrite local file with remote version
   - Abort: Exit with code 3

This ensures users are always in control of their own device's configuration.

## Why This Design?

### Handles IP Changes
- Devices identified by hostname, not IP
- Git remotes use hostnames
- SSH config uses hostnames

### Decentralized
- No single point of failure
- No need for a central server
- Works with any network topology

### Robust
- Handles one-way SSH connections
- Handles offline devices
- Handles partial connectivity
- User controls own config

### Automatic
- No manual git remote management
- No manual IP updates
- Sync happens automatically

### Simple
- Single config location
- Standard git operations
- No custom protocols or formats
