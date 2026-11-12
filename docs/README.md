# regroup — Documentation

This directory contains the technical documentation for the **regroup** project.

## Overview

**regroup** is a tool that finds devices by their SSH host key fingerprint and updates `~/.ssh/config`. Devices announce themselves via signed UDP beacons, and regroup listens continuously, prompting for each new device.

Configuration is stored in `~/.regroup-config/git/` (a git repository) where each device owns a directory under `devices/<name>/` holding its state and lists.

## Documentation Structure

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | Big picture, data flow, how regroup works |
| [Navigation](navigation.md) | TUI navigation model: Esc/arrows/Enter context hierarchy |
| [Storage Rules](storage.md) | Configuration structure, write ownership, conflict resolution |
| [Modules](modules/) | Module-specific documentation |
| [Development](development.md) | Coding rules, conventions, build & test |

### Modules

- [config](modules/config.md) — Configuration types, loading, validation
- [discovery](modules/discovery.md) — Beacon format, signing, verification
- [git](modules/git.md) — Repository management, sync, remotes
- [ssh](modules/ssh.md) — SSH key handling, known_hosts verification
- [ui](modules/ui.md) — Terminal UI (TUI and plain fallback)
- [interactive](modules/interactive.md) — Main loop, state management

## Quick Links

- [User-facing documentation](../README.md) — Usage, installation, commands
- [Developer notes](../src/README.md) — Technical deep-dive (legacy, see docs/ for updated)
- [Source code](../src/) — Browse the implementation

## Getting Started

1. Read the [user guide](../README.md) for usage information
2. See [Architecture](architecture.md) to understand how regroup works
3. Check [Development](development.md) for coding standards and build instructions

---

*This documentation is a work in progress. Contributions welcome!*
