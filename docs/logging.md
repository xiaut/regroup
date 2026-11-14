# Logging Policy

This page defines what regroup logs and when. It applies to every code path:
the interactive TUI, its plain fallback, the internal `receive-*`
subcommands, and any future front-end (CLI or library consumer), which all
report through the same `regroup::logging` sink.

## Principles

- **There is no verbose mode.** No `-v` flag, no `v` key toggle. Information the
  user needs is always logged; nothing important is hidden behind a flag.
- Logs go to the **Logs pane** in the full-screen TUI (kept up to `LOG_CAP`
  entries) and to **stdout** in plain mode, before the UI starts, or from a
  front-end that installs no sink of its own (library use).
  Device-specific messages go through `log_device()` with a `Level`
  (`Info` / `Success` / `Warning` / `Security`); security messages additionally
  appear in the Logs pane and flash longer so they cannot be missed.
- Messages are in English and carry context: which device, remote, host, or
  file is concerned.

## Rule 1 — Announce password prompts before they happen

Whenever regroup runs an operation that may ask the user for a password or
credentials (`ssh`, `git pull`/`push` against an authenticated remote — note
that `ssh-keyscan` never prompts: it only fetches host keys without
authenticating), a log line must explain what is about to happen **just
before** the operation starts. In full-screen mode the external command's prompt is visible
(stderr is passed through in direct output mode), but without the log line the
user cannot tell *why* a prompt appeared.

When adding a new call to an external command that can prompt, add the log
line first.

## Rule 2 — Log every non-standard path

Any deviation from the normal flow must produce a log line. "Non-standard"
includes at least:

- **Fallbacks and detection failures** — e.g. host key not found in the
  standard files, falling back to `ssh-keyscan` on localhost; no local network
  interfaces; cannot bind the announce socket; device has no SSH server and
  will not announce.
- **Dry-run skips** — every action not executed because the front-end runs
  with `dry_run` enabled is logged as `  - (dry run - ... skipped)` or
  `[DRY RUN] ...`.
- **Skipped or rejected work** — e.g. `  - Skipped <name>.` when the user
  ignores a device; beacons ignored for the rest of the run.
- **Warnings** — config validation problems, beacon signing failures — prefixed
  `WARNING: ...`.
- **Security events** — `Level::Security` via `log_device()` or
  `[!] SECURITY: ...`: invalid announced host key, signing key rotation
  (possible spoofing/MITM), fingerprint mismatch.
- **Errors** — git sync errors (`[sync] ...`), stderr of `ssh` / `ssh-keyscan` /
  `ssh-keygen`, command spawn failures, unreachable devices
  (`Device <name> unreachable`).
- **No-ops** — `Nothing to wipe: ... does not exist.`, no git remotes
  configured, nothing to commit, connection already exists.
- **Per-remote sync results** — `  - pulled from <remote>` /
  `  - pushed to <remote>` are always logged for every remote.

When adding a branch that is not the normal path, add a log line in that
branch.

## Message style

**Sub-lines use a single symbol: `  - `** — every detail line under an
operation's header line (e.g. under `Syncing git...`) starts with `  - ` and
nothing else. No `.` / `+` / `!` variants; severity is carried by the parent
line or by `Level`, never by the sub-line symbol.

| Situation | Prefix / level |
|-----------|----------------|
| Warning | `WARNING: ...` |
| Security event | `[!] SECURITY: ...` or `Level::Security` |
| Dry-run simulation | `[DRY RUN] ...` |
| Dry-run skip | `  - (dry run - ...)` |
| External command output | `<tool>: <line>` (e.g. `ssh-keyscan: ...`) or `ssh <user>@<host>: <line>` |
| Sub-line of an operation | `  - ...` (the only sub-line symbol) |
