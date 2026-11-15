# internal Module — remote API subcommands (binary)

`internal` holds the subcommands regroup runs **on the target device over
SSH** when the TUI drives a remote operation:

```
regroup receive-add-local
regroup receive-install-key --source-user U --source-device D --public-key K --connection-id U->V@D
regroup receive-remove-key  --connection-id U->V@D
```

**Naming rule: every remote API command starts with `receive-`.** A
subcommand whose name starts with `receive-` is meant to be invoked on a
remote device (`ssh user@device regroup receive-...`); it may also be run
locally. Anything else (`remove`, `wipe`, `setup-cd`, ...) is a local CLI
command and must not start with `receive-`.

The TUI invokes them as `ssh user@device regroup receive-...` (source
side: `tui/key_actions.rs`, built on `ops::keys`); the add-user flow runs
exactly one of `receive-add-local` XOR `receive-install-key` per added
user, then a device-scoped git sync.

- `receive-add-local` adds the target device's own localhost to its
  shared configuration (device file + current user file, committed) —
  the same code the TUI localhost row (`a`) runs locally.
- `receive-install-key` **starts by running the same add-local step**
  (a key install implies the target device is in its own shared
  configuration — shared code, no duplication), then records the
  connection and installs the key in `authorized_keys`.

This is not a public CLI: there is no help text and no other subcommand.
Unknown arguments exit with code 2, handler errors exit 1, success exits 0.

> **Target of the upcoming refactor:** these handlers move from `src/cli/`
> to `src/internal/` in the binary crate, using `regroup::ops::keys` for the
> actual work (authorized_keys, connection files, git commit). The rest of
> the former `cli` module becomes the library's `ops` layer (see
> [ops.md](ops.md)).
