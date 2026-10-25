# Configuration Architecture Update

## Summary

**Configuration is now always stored in `~/.regroup-config/`** - a git repository where each device has its own JSON file in the `devices/` directory.

## Key Features

### 1. Automatic Git Remote Management

- **Automatic addition**: When a new device is detected in the config, a git remote is automatically added
- **Automatic removal**: When a device is removed from config, its remote is automatically removed
- **Remote URL format**: `user@hostname:~/.regroup-config`
- **Self-exclusion**: A device never adds itself as a remote

### 2. Decentralized Sync

- **`git pull --all`**: Used with `-r` flag to pull from all remotes before discovery
- **`git push --all`**: Used after updating SSH config to push to all remotes
- **Graceful failures**: One-way SSH connections are handled with warnings only

### 3. Sync Workflow

```
1. On startup with -r flag:
   - git pull --all (pulls from all remotes)
   - Load all device configs
   
2. After discovery:
   - Update SSH config for each discovered device
   - git push --all (pushes to all remotes)
   - Update git remotes to match current device list
   
3. Continuous:
   - Every run refreshes git remotes
   - Stale remotes are removed
   - New devices get remotes added
```

### 4. Configuration Structure

```
~/.regroup-config/
├── .git/                      # Git repository
├── settings.json             # Global settings (optional)
└── devices/
    ├── ubuntu-MacBookPro.json # Device 1 (this device)
    ├── ordi.json             # Device 2 (remote added)
    └── nas.json              # Device 3 (remote added)
```

### 5. Device File Format

```json
{
  "name": "hostname",           # Required
  "fingerprint": "SHA256:...",  # Required
  "user": "username",           # Optional (default: current user)
  "port": 22                    # Optional (default: 22)
}
```

### 6. Global Settings Format

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

**Device A:**
```bash
./regroup
# Creates config, initializes git
```

**Device B:**
```bash
./regroup
# Creates config, adds A as remote, pushes to A
```

**Device A:**
```bash
./regroup -r
# Pulls from B, adds B as remote, now both sync bidirectionally
```

### Manual sync when automatic fails

```bash
# On any device
cd ~/.regroup-config
git pull --all
git push --all
```

### Adding a device

Just create the device file and commit. Remotes are updated automatically on next run:

```bash
cat > ~/.regroup-config/devices/newdevice.json << EOF
{
  "name": "newdevice",
  "fingerprint": "SHA256:..."
}
EOF
git add .
git commit -m "Add newdevice"
./regroup  # Will add newdevice as remote and push to it
```

## Technical Details

### Git Remote Management Function

The `refresh_git_remotes()` function:
1. Gets list of existing remotes
2. Removes remotes that don't correspond to any machine (except self)
3. Adds remotes for all machines (except self)
4. Updates existing remotes if URL changed

### Sync Functions

- `git_pull_all()`: Runs `git pull --all --no-rebase` and collects errors
- `git_push_all()`: Runs `git push --all --porcelain` and collects errors
- `sync_with_remotes()`: Combines pull and push, returns all errors

### Error Handling

All git operations that might fail (due to one-way SSH) are:
- Wrapped in error collection
- Reported as warnings (not errors)
- Continue execution even if some remotes fail

This ensures the system works even with partial connectivity.

## Why This Design?

### Decentralized
- No single point of failure
- No need for a central server
- Works with any network topology

### Robust
- Handles one-way SSH connections
- Handles offline devices
- Handles partial connectivity

### Automatic
- No manual git remote management
- Remotes always match device list
- Sync happens automatically on every run

### Simple
- Single config location
- Standard git operations
- No custom protocols or formats
