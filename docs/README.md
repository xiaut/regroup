# regroup — Documentation

This directory contains the technical documentation for the **regroup** project.

## Overview

**regroup** is a tool that finds devices by their SSH host key fingerprint and updates `~/.ssh/config`. Devices announce themselves via signed UDP beacons, and regroup listens continuously, prompting for each new device.

Configuration is stored in `~/.regroup-config/git/` (a git repository) where each device owns a directory under `devices/<name>/` holding its state and lists.

> **Target of an upcoming refactor:** these docs describe the target
> architecture — a `regroup` library crate with a UI-agnostic feature layer
> (`ops`), plus a thin binary that keeps the TUI and the internal `receive-*`
> subcommands while the full CLI subcommands are dropped. Where the current
> code differs, the docs win; the code is being migrated to them (same
> convention as [Navigation](navigation.md)).

The target layout separates the **feature layer** (`ops`) from the **data &
protocol layers** (`config`, `discovery`, `git`, `ssh`, `signing`), and
splits every module into small files — one file per responsibility,
~500-800 lines max — so the repository can be understood file by file,
without reading huge files. See
[Architecture](architecture.md#crate-layout-and-layering-target) for the
layering and file-splitting rules, and
[File Structure](development.md#file-structure) for the full file tree.

## Documentation Structure

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | Big picture, data flow, how regroup works |
| [Navigation](navigation.md) | TUI navigation model: Esc/arrows/Enter context hierarchy |
| [Storage Rules](storage.md) | Configuration structure, write ownership, conflict resolution |
| [Logging Policy](logging.md) | What is logged and when: no verbose mode, password-prompt and non-standard-path rules |
| [Modules](modules/) | Module-specific documentation |
| [Development](development.md) | Coding rules, conventions, build & test |

### Modules

Library crate (`regroup`, `src/lib.rs`):

- `logging` — UI-agnostic global log sink shared by every layer
- [config](modules/config.md) — Configuration types, loading, validation
- [discovery](modules/discovery.md) — Beacon format, signing, verification
- [git](modules/git.md) — Repository management, sync, remotes
- [ssh](modules/ssh.md) — SSH key handling, known_hosts verification
- [ops](modules/ops.md) — The feature layer: every capability, UI-agnostic
  (devices, sync, keys, shell, selftest, help)

Binary crate (`src/main.rs`) front-ends:

- [internal](modules/internal.md) — `receive-*` subcommands run over SSH by the key actions
- [ui](modules/ui.md) — Terminal UI (TUI and plain fallback)
- [interactive](modules/interactive.md) — Main loop, state management
- `tui` — Key actions, connections browser

## Quick Links

- [User-facing documentation](../README.md) — Usage, installation, commands
- [Developer notes](../src/README.md) — Technical deep-dive (legacy, see docs/ for updated)
- [Source code](../src/) — Browse the implementation

## Getting Started

1. Read the [user guide](../README.md) for usage information
2. See [Architecture](architecture.md) to understand how regroup works
3. Check [Development](development.md) for coding standards and build instructions
