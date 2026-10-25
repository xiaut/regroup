# Configuration Architecture Update

## Summary

**Configuration is now always stored in `~/.regroup-config/`** - a git repository where each device has its own JSON file in the `devices/` directory.

The old `regroup.json` single-file configuration is **completely removed**. This is a breaking change that fixes the scalability and sync issues.

## Key Changes

### 1. Fixed Configuration Location

**Before:** Config file path could be specified with `-c` flag, defaulted to `./regroup.json`
**After:** Config is **always** at `~/.regroup-config/` - no `-c` flag, no other options

### 2. Per-Device Configuration Files

**Before:** Single file with all machines in a `machines` array
```json
{
  "machines": [
    {"name": "ordi", "fingerprint": "...", "user": "...", "port": 22},
    {"name": "nas", "fingerprint": "...", "user": "...", "port": 22}
  ]
}
```

**After:** Separate file for each device
```
~/.regroup-config/devices/ordi.json
~/.regroup-config/devices/nas.json
```

### 3. Auto-Registration

Each device automatically creates its own configuration file on first run with:
- Auto-detected hostname
- Auto-detected SSH fingerprint
- Current user
- Default port (22)

### 4. Git Integration

The `~/.regroup-config/` directory is automatically initialized as a git repository on first run. You can:
- Use `./regroup -r` to pull latest changes before running
- Manually run `git pull` in `~/.regroup-config/`
- Commit and push changes to a remote repository

### 5. Removed Command-Line Options

- `-c REPO` / `--config REPO`: **Removed** - config location is fixed at `~/.regroup-config/`

### 6. New Command-Line Options

- `-r` / `--refresh`: Pull latest changes from git repo before running

## Migration Guide

### For existing users:

The old `regroup.json` format is **no longer supported**. You must migrate:

1. **Backup your old config:**
   ```bash
   cp regroup.json regroup.json.bak
   ```

2. **Run regroup to create new structure:**
   ```bash
   ./regroup test
   ```
   This creates `~/.regroup-config/` with a device file for your current machine.

3. **Add other devices manually:**
   For each machine in your old `regroup.json`, create a file in `~/.regroup-config/devices/`:
   
   Example: For a machine named "ordi" with fingerprint "SHA256:abc..." and user "matthias":
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
   If you had custom settings (udp_port, wait_secs, batch_size, subnets), add them to `settings.json`:
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

5. **Commit changes:**
   ```bash
   cd ~/.regroup-config
   git add .
   git commit -m "Migrated from old config"
   ```

6. **Sync to other devices:**
   On other devices, either:
   - Copy the directory: `cp -r ~/.regroup-config user@other-device:~/.regroup-config`
   - Clone from a remote: `git clone your-repo-url ~/.regroup-config`

## Benefits

1. **No merge conflicts**: Each device writes to its own file
2. **Predictable location**: Always `~/.regroup-config/` - no confusion about where config is
3. **Easy distribution**: Use git or any file copy method
4. **Auto-discovery**: Devices self-register with minimal user input
5. **Clean separation**: Global settings separate from device configs

## Breaking Changes

- Old `regroup.json` format is **not supported**
- `-c` flag is **removed** - config location is fixed
- Config directory is **always** `~/.regroup-config/`

## Technical Details

### File Structure

```
~/.regroup-config/
├── .git/                     # Git repository
├── settings.json            # Global settings (optional)
└── devices/
    ├── <hostname1>.json     # Device 1 config
    ├── <hostname2>.json     # Device 2 config
    └── <hostname3>.json     # Device 3 config
```

### Device File Format

```json
{
  "name": "hostname",           # Required
  "fingerprint": "SHA256:...",  # Required
  "user": "username",           # Optional (default: current user)
  "port": 2222                  # Optional (default: 22)
}
```

### Global Settings Format

```json
{
  "udp_port": 4242,             # Optional (default: 4242)
  "wait_secs": 10,             # Optional (default: 10)
  "batch_size": 64,            # Optional (default: 64)
  "subnets": ["192.168.43"]   # Optional (default: [])
}
```
