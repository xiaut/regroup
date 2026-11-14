# internal Module — internal subcommands (binary)

`internal` holds the two subcommands regroup runs **on the target device over
SSH** when a user installs or removes an SSH key from the TUI:

```
regroup receive-install-key --source-user U --source-device D --public-key K --connection-id U->V@D
regroup receive-remove-key  --connection-id U->V@D
```

The TUI key actions invoke them as `ssh user@device regroup receive-...`
(source side: `tui/key_actions.rs`, built on `ops::keys`).

This is not a public CLI: there is no help text and no other subcommand.
Unknown arguments exit with code 2, handler errors exit 1, success exits 0.

> **Target of the upcoming refactor:** these handlers move from `src/cli/`
> to `src/internal/` in the binary crate, using `regroup::ops::keys` for the
> actual work (authorized_keys, connection files, git commit). The rest of
> the former `cli` module becomes the library's `ops` layer (see
> [ops.md](ops.md)).
