# ops Module — the feature layer

`ops` is regroup's feature layer: every user-facing capability lives here,
behind functions that know nothing about terminals. The TUI front-end
(`interactive`, `tui`), a future CLI, or any Rust program embedding the
`regroup` library all call the same functions.

> **Target of the upcoming refactor:** today these functions live in
> `src/cli/` inside the binary; the refactor moves them into the library as
> `regroup::ops`, with the log sink in `regroup::logging` and
> `process::exit` replaced by returned `Result`s.

## Module Structure

```
ops/
├── mod.rs
├── devices.rs   # add / remove / wipe devices, name-conflict helpers
├── sync.rs      # git sync of the config repo, signing-key trust
├── keys.rs      # connection key pairs, authorized_keys, connection IDs
├── shell.rs     # setup_cd: install the 'regroup-cd' shell function
├── selftest.rs  # environment checks as a library feature
└── help.rs      # help text shared with the TUI '?' panel
```

## Rules

- **No front-end dependencies.** `ops` may use `logging`, `config`,
  `discovery`, `git`, `ssh`, `signing` — never `ui` / `tui` / `interactive`.
- **No `process::exit`.** Return `Result<_, String>`; the caller decides how
  to report failures and which exit code to use.
- **Report through `regroup::logging`** (`log`, `log_device`, `warn`): the
  sink is stdout until a front-end installs its own (see
  [Logging policy](../logging.md)).

## devices.rs

| Function | Purpose |
|----------|---------|
| `add_device_to_config(home, ssh_config, beacon, name, new_count) -> Option<Device>` | Verify the announced host key, write the placeholder device file to the `tmp/` overlay (never committed), update `~/.ssh/config` |
| `remove_device(home, ssh_config, device, last_ip) -> bool` | Delete the device directory in both proxy layers (`git/` deletion committed, propagates via sync; `tmp/` copy removed) + local traces |
| `remove_local_traces(home, ssh_config, device, last_ip)` | `~/.ssh/config` block, known_hosts entries (last known IP), git remote |
| `wipe(home, ssh_config, devices) -> Result<(), String>` | Remove all regroup `Host` blocks, then delete `~/.regroup-config` entirely |
| `device_name_taken` / `find_name_conflicts` / `unique_device_name` | Name-collision checks; `NameConflict` = `RegroupDevice` / `DeviceFileOnDisk` / `SshConfigBlock` |

## sync.rs

| Function | Purpose |
|----------|---------|
| `sync_git(home, temp_users)` | Configure a git remote for every known device (except self), then pull from and push to each remote; per-remote results are always logged |
| `trust_new_signing_key(home, beacon)` | After a signing-key rotation is accepted, pull the device's authoritative `device.json` |

## keys.rs — connection key management

`generate_key_pair`, `get_public_key`, `install_authorized_key`,
`remove_authorized_key`, `is_reachable`, `get_config_dir`, `commit_changes`,
`parse_connection_id`, `build_connection_id`, `get_local_device_name`,
`get_current_user`.

Used on both sides of a key installation: by the internal `receive-*`
subcommands on the target device (see [internal.md](internal.md)), and by the
TUI key actions on the source device, which then run
`ssh user@device regroup receive-install-key ...` / `receive-remove-key ...`.

## shell.rs

`shell_rc_file(home)` and `setup_cd(home) -> Result<PathBuf, String>` —
install or refresh the `regroup-cd` shell function in `~/.zshrc` /
`~/.bashrc`. Reachable from the TUI (key `c` in the Added panel).

## selftest.rs

`run() -> SelftestReport` — checks that `ssh-keyscan`, `ssh-keygen` and `git`
are present, and that regroup's fingerprint computation agrees with
`ssh-keygen -lf`. The report exposes `tools_ok`, `fingerprint_ok` and
`ok()`; **there is no exit code** — the former `regroup selftest` subcommand
is dropped from the binary and this library feature replaces it.

## help.rs

`usage_text() -> &'static str` — the help text shown by the TUI `?` panel
(and printed by the plain fallback's help). It describes the interactive
commands only; the dropped CLI subcommands are not listed.
