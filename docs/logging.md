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

## Rule 3 — One log stream; the CLI shows only the CLI passes

There is a **single log stream**. The **Logs pane of the full-screen TUI
always shows it in full**. Every action is delimited in the stream by two
markers: `TUI -> CLI (<action>)` when the action starts and
`CLI -> TUI (<action>)` when it ends. Two kinds of action delimitation
exist:

- A **temporary CLI pass** (`Ui::with_clean_terminal`) additionally
  switches the screen: it is the only time the stream is also printed to
  the CLI — so **the CLI shows exactly the lines emitted during passes**,
  and **everything shown in the CLI during a pass appears in the Logs
  pane** when the TUI resumes. Reserve it for actions that run a
  subprocess needing the live terminal — git sync, signing-key trust
  (pulls from the device), SSH key install/remove — because git pull/push
  and ssh may prompt for a password, which only works with the screen
  switched away.
- A **logged action** (`Ui::logged_action`: device add/remove, wipe) only
  writes the two markers: the TUI stays on screen, nothing is printed to
  the CLI, there is no "press Enter to return" pause — these actions only
  log through the stream and run no live-output subprocess.

Mechanics:

- The pass window (the operation itself and the "press Enter to return"
  pause, until the TUI is re-initialized) is bracketed by a RAII guard
  (`DirectOutputGuard`) that switches the sink to direct output mode and
  restores it even on panic. In this mode `log()` / `log_device()` print
  to stdout in addition to being recorded in the UI channel.
- External commands whose output belongs to the stream run through
  `ui::run_logged` (git, ssh, ...): in direct output mode their stdin is
  inherited and their stdout/stderr are forwarded live to the CLI —
  interactive prompts work, the output appears in the CLI — and recorded
  into the UI channel; outside a pass (TUI on screen, or a logged
  action) their output is captured and logged line by line.

Consequences for new code:

- Always log through `log()` / `log_device()` — never send `UiEvent`s to
  the channel directly outside `logging.rs` (`record()` is for output
  already shown live in the CLI during a pass). `pulse_device` is the only
  channel-only event and is a row animation, not a log line.
- Run external commands through `ui::run_logged`, never a bare `Command`
  with inherited or captured streams, so their output lands in the stream
  in both modes.

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
