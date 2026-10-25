# Configuration Architecture Update

## Summary

Updated the configuration system from a single `regroup.json` file to a **distributed git repository** approach where each device has its own configuration file.

## Key Changes

### 1. New Configuration Structure

**Old:** Single `regroup.json` file with all machines
```
regroup.json
```

**New:** Git repository with per-device files
```
regroup-config/
├── settings.json          # Global settings (optional)
└── devices/
    ├── <hostname1>.json
    ├── <hostname2>.json
    └── <hostname3>.json
```

### 2. Auto-Registration

Each device automatically creates its own configuration file on first run with:
- Auto-detected hostname
- Auto-detected SSH fingerprint
- Current user
- Default port (22)

This happens transparently when a device's config file is missing.

### 3. Git Integration

The configuration repository can be:
- A local git repository
- A remote git repository (cloned on first use)
- A plain directory (git features optional)

Git operations supported:
- `git clone` - for initializing from a remote
- `git pull` - for refreshing before runs (`-r` flag)
- `git add` + `git commit` - for saving new device configs

### 4. New Command-Line Options

- `-c REPO` / `--config REPO`: Specify config repository path or URL (default: `./regroup-config`)
- `-r` / `--refresh`: Pull latest changes from git repo before running

### 5. Backward Compatibility

The old `regroup.json` single-file format is still supported for reading, but new installations should use the distributed approach.

## Migration Guide

### For existing users with `regroup.json`:

1. Create a new config repository:
   ```bash
   mkdir regroup-config
   cd regroup-config
   git init
   mkdir devices
   ```

2. Convert your old config to new format:
   - Copy machines from `regroup.json` to individual files in `devices/`
   - Example: Machine named "ordi" → `devices/ordi.json`
   - Copy global settings to `settings.json`

3. On each device, run:
   ```bash
   ./regroup -c /path/to/regroup-config
   ```

### For new users:

Simply run:
```bash
./regroup -c /path/to/regroup-config
```

The first device will:
1. Create the config directory
2. Initialize a git repository
3. Create its own device file
4. You can then push to a shared location and clone on other devices

## Technical Details

### Device File Format

Each device file (`devices/<name>.json`) contains:

```json
{
  "name": "hostname",
  "fingerprint": "SHA256:...",
  "user": "username",
  "port": 22
}
```

All fields except `name` and `fingerprint` are optional and will use defaults:
- `user`: Current system user
- `port`: 22 (omitted from file if default)

### Global Settings Format

Optional `settings.json`:

```json
{
  "udp_port": 4242,
  "wait_secs": 10,
  "batch_size": 64,
  "subnets": ["192.168.43", "192.168.1"]
}
```

### Workflow

1. **On first device:**
   - Run `./regroup -c ./regroup-config`
   - Device auto-creates its config file
   - Git repository is initialized
   - Commit and push to shared location

2. **On subsequent devices:**
   - Clone the config repository
   - Run `./regroup -c ./regroup-config`
   - Device auto-creates its config file (if missing)
   - Commit and push changes

3. **Sync across devices:**
   - Use `./regroup -c ./regroup-config -r` to pull latest changes
   - Or manually `git pull` in the config repository

## Benefits

1. **No merge conflicts**: Each device writes to its own file
2. **Easy distribution**: Use git to sync configurations
3. **Auto-discovery**: Devices self-register with minimal user input
4. **Scalable**: Works well with many devices
5. **Flexible**: Can use git or just a plain directory

## Implementation Notes

- Uses `std::process::Command` for git operations (no external dependencies)
- Git is optional - tool works with plain directories
- JSON files are human-readable and editable
- All existing functionality preserved (UDP beacons, fingerprint matching, SSH config updates)
