# ui Module

The `ui` module provides the terminal user interface for interactive mode.

> **Target of the upcoming refactor:** `ui` is a front-end of the **binary**
> crate. The log sink (`log`, `log_device`, `pulse_device`, `restore`,
> `Level`, `UiEvent`) moves to the library as `regroup::logging`
> (`src/logging.rs`); `ui::mod` re-exports it and `ui::types` re-exports
> `Level`/`UiEvent`, so call sites keep working.

## Overview

This module implements two UI backends with the same interface:

1. **Full-screen TUI** (ratatui): A rich terminal interface with device panels, Logs panel, and question display
2. **Plain fallback**: A simple line-based interface for non-terminal output (piped to files or other commands)

The module automatically selects the appropriate backend based on whether stdout is a terminal.

## Module Structure

```
ui/                          # binary front-end
├── mod.rs                   # Module root; re-exports regroup::logging
├── types.rs                 # UI types; re-exports Level, UiEvent from regroup::logging
├── interface.rs             # Ui enum (Tty/Plain) with unified interface
├── tty.rs                   # TtyUi: full-screen ratatui implementation
├── plain.rs                 # PlainUi: line-based fallback implementation
└── render.rs                # Rendering functions and styling utilities

logging.rs (src/logging.rs)  # library: the log sink itself (moved out of ui/)
```

## UI Selection

The UI backend is selected automatically in `Ui::start()`:

```rust
pub fn start() -> Ui {
    if io::stdout().is_terminal() {
        // Create TtyUi with full-screen interface
        Ui::Tty(Box::new(TtyUi::new(...)))
    } else {
        // Create PlainUi for piped output
        Ui::Plain(PlainUi::new())
    }
}
```

## Public Types

### `UiEvent`

Messages sent through the logging channel:

```rust
pub enum UiEvent {
    Global(String),           // General message for Logs pane
    Device {                 // Device-specific message
        key: String,           // SSH public key (identifies device)
        name: String,
        level: Level,
        msg: String,
        detail: Option<String>, // Long form for plain mode
    },
    Pulse { key: String },    // Beacon received (animate row)
}
```

### `Level`

Priority levels for device messages:

```rust
pub enum Level {
    Info,      // Cyan, steady - something happened, no action needed
    Success,   // Green background, steady - state improved
    Warning,   // Yellow, blinking - attention needed
    Security,  // Red, blinking - possible spoofing/MITM
}
```

### `Device`

Represents a device for display in panels:

```rust
pub struct Device {
    pub key: String,      // SSH public key
    pub name: String,
    pub ip: Option<String>,
    pub port: u16,
}
```

### `Question`

Current question being asked of the user:

```rust
pub enum Question {
    Idle,
    AddDevice { name: String, ip: String, fingerprint: String },
    ConfirmDeviceKey { name: String, ip: String, fingerprint: String },
    TrustKey { name: String },
    Rename { name: String, suggested: String },
    Delete { name: String },
    Wipe { count: usize },
    SetupCd { file: String },
    // SSH key management questions
    AddTempUser { device: String }, // Users panel, input handled like Rename
}
```

### `Action`

User decisions translated from key presses:

```rust
pub enum Action {
    Quit,
    Sync,
    AskAdd,         // Arm the key-fingerprint confirmation of the selected detected device
    Add,            // Add the detected device after its fingerprint was confirmed
    Ignore,         // Ignore selected device
    AddCurrent,     // Add front of pending queue (plain mode)
    SkipCurrent,    // Skip front of pending queue (plain mode)
    TrustKey,
    RejectKey,
    Rename(String),
    SkipRename,
    DeleteMode,
    SelectPrev,
    SelectNext,
    DeleteConfirm,
    DeleteCancel,
    WipeMode,
    WipeConfirm,
    WipeCancel,
    SetupCdMode,
    SetupCdConfirm,
    SetupCdCancel,
    ConnectionsMode,
    SwitchToUsers,
    BackToAdded,
    UsersSelectPrev,
    UsersSelectNext,
    AddTempUser,
    TempUserName(String),
    CancelTempUser,
    RemoveTempUser,
    Help,
}
```

## Logging Functions

These live in the library (`regroup::logging`, `src/logging.rs`) and are
re-exported by `ui`:

### `log()`

Display a general message:

```rust
pub fn log(msg: &str)
```

- In full-screen mode: appears in the Logs pane
- In plain mode: printed to stdout

### `log_device()`

Display a device-specific message:

```rust
pub fn log_device(
    key: &SshPublicKey,
    name: &str,
    level: Level,
    msg: &str,
    detail: Option<&str>,
)
```

- In full-screen mode: animated status on the device's row
- In plain mode: printed as "name: msg" or detail
- Security messages also appear in the Logs pane

### `pulse_device()`

Pulse a device's row (beacon received):

```rust
pub fn pulse_device(key: &SshPublicKey)
```

- Full-screen only: row lights up and fades out
- Visual feedback that a beacon was received

### `restore()`

Best-effort terminal restoration:

```rust
pub fn restore()
```

- Restores terminal from raw mode
- Leaves alternate screen
- Called on panic/exit to avoid leaving terminal in bad state

## Full-Screen UI (TtyUi)

The TUI interface uses **ratatui** (formerly tui-rs) with **crossterm** for terminal control.

### Layout

```
┌──────────────────────────────────────────────┐
│  [Devices — 1 added, 1 detected]             │
│  ┌────────────────────────────────────────┐  │
│  │ Name               │ IP      │Port│Status │  │
│  │--------------------│---------│----│-------│  │
│  │ ── localhost ──                         │  │
│  │ myhost             │ 1.2.3.4 │ 22 │this dev│  │
│  │ ── added ──                             │  │
│  │ dev2               │ 5.6.7.8 │ 22 │known  │  │
│  │ ── detected ──                          │  │
│  │ dev1               │ 9.8.7.6 │ 22 │detected│  │
│  └────────────────────────────────────────┘  │
├───────────────[ Logs ]──────────────────────┤
│  Device dev1 detected                       │
│  Device dev2 IP updated                     │
│  (tail of the log — fixed 6-row strip)      │
│                                             │
├─────────────────────────────────────────────┤
│  [Status]                                   │
│  1 device(s) detected — add it?            │
│  [Up/Right] devices   [Down] logs          │
│  [q] quit   [?] help                        │
└─────────────────────────────────────────────┘
```

### Panels

- **Devices panel** (top, full width, green): one list, each section
  opening with a divider row in the same style. The list starts with
  `── localhost ──` and the local device row (the one regroup runs on),
  then `── added ──` and the added devices (already in the
  configuration, with their last known IP), then `── detected ──` and
  the detected devices (announced but not accepted).
  A **Users sub-panel** is nested inside this panel for the selected added
  device; it lists the device's users with their status (temporary — to
  confirm / confirmed / SSH key installed). It is a sub-context of the
  device, not a device, not a sibling panel: `Right`/`Enter` from the
  Devices pane with an added row selected enters the sub-panel (first
  user selected) — only when the device has at least one user, otherwise
  the sub-panel is not reachable and the keys are not offered; `Left`/`Esc`
  returns to the Devices pane. Device+user actions (install/remove SSH key) live there and only
  appear for a confirmed user on a non-local device; a temporary user
  only offers its own user-specific action (`x` remove) — the git sync
  that brings the device's declared users is a global action on the main
  menu (`s`). Adding a user (`a`) is device-specific and lives on the
  added device row, not here. On the localhost device the pane also ends
  with an `── external ──` group listing the incoming connections
  targeting the current user (revoke with `r`). See
  [navigation.md](../navigation.md).
- **Logs strip** (bottom of the main area, always visible): tail of the
  general log
- **Question block** (bottom): Current question and available actions

The **Logs pane is always visible** as a fixed-height strip (6 rows,
borders included) at the bottom of the main area, above the question
block; the Devices pane takes the rest of the main area. The strip is
dropped when the main area is shorter than 12 rows, and hidden while the
help panel is open or while the Logs pane is fullscreen. `Down` from the
overview focuses the strip, then `Down`/`Enter` opens the fullscreen Logs
panel; scrolling the log still requires that fullscreen panel.

The Logs pane always shows the **tail** of the log: the newest lines sit
on the pane's last content row (the log is bottom-anchored, with blank
rows above when it is shorter than the pane). Scrolling up is only
possible inside the fullscreen Logs pane and never affects the overview
or the strip — they always follow the tail.

## Plain UI

For non-terminal output (piped), the plain UI provides a line-based interface:

```
[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, h=help]: 
Add device1 (192.168.1.100) to config? [y/N/q/s]: y
Add device2 (192.168.1.101) to config? [y/N/q/s]: n
[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, h=help]: q
```

## Key Bindings

In the full-screen ui, keys are **scoped to the selected context** (see
`pane_keys` in `src/ui/types.rs`): the hint line always shows exactly the
keys that work in the current pane state, and a key that is not listed is
not handled. Navigation keys and `?` help are available in every pane
state; `?` is the single key that works everywhere.

### Main menu (overview)

The main menu offers the **global actions** — everything not specific to
a device or a user — plus navigation:

| Key | Action |
|-----|--------|
| `s` | Sync git, unscoped: every device × every user (pull/push) |
| `w` | Wipe configuration |
| `c` | Setup regroup-cd shell function |
| `q` | Quit (only offered here) |
| `Up`/`Right` | Focus the Devices panel |
| `Down` | Focus the Logs strip |
| `?` | Show the help panel (Esc closes it) |

### In Devices Panel (added row selected)

Only device-specific actions (the strict globals stay on the main menu) —
except sync, which is **context-scoped**: here `s` syncs this device only,
every user of it:

| Key | Action |
|-----|--------|
| `d` | Delete selected device |
| `s` | Sync git scoped to this device: every user of it (pull/push) |
| `a` | Add a user for the device (`a` is free on this row; adding a user is device-specific — no user needs to be selected). The user is temporary until a sync or a successful key install confirms it |
| `Right` or `Enter` | Open the Users sub-panel of the selected device (only when the device has at least one user — with no users the pane is not reachable) |

### In Devices Panel (localhost row selected)

| Key | Action |
|-----|--------|
| `a` or `r` | Add / remove the local device to/from the shared configuration (depending on its membership) |
| `Right` or `Enter` | Open the Users sub-panel of the local device (only when the device has at least one user) |

No add-user key on the localhost row: a temporary user configures an
outgoing connection, and connecting to the current device is nonsense.

### In Devices Panel (detected row selected)

| Key | Action |
|-----|--------|
| `a` | Arm the confirmation of the device's SSH key fingerprint — a modal shows the fingerprint; the device is added only after `y` confirms it (no silent acceptance) |
| `n` | Ignore selected device |

### In Users Panel (a user selected)

Only user-specific actions, in two layers — the user layer and, on top of
it, the key layer. The same keys are offered on every user row of an added
device: temporary is not a special kind of user (a user added with `a` is
temporary until a sync or a successful key install confirms it):

| Key | Action |
|-----|--------|
| `d` | Set the selected user as the device's default user (device-owned data — the one exception to "on localhost you only edit your own data") |
| `s` | Sync git, context-scoped: every device × every user, but the selected device as the selected user only (added device only — not offered on the localhost Users pane: the local device has no remote, so sync is unavailable on the current user) |
| `x` | Remove the selected user (tmp/ entry, or declared user file committed so the removal propagates) |
| `i` / `r` | Install / remove SSH key to `user@device` (added device; a successful install on a temporary user confirms it; recorded as an outgoing connection file) |
| `r` | Revoke — **only on an `── external ──` row of the localhost device**: remove the incoming key from your `authorized_keys` and delete the connection file, locally |
| `Left` or `Esc` | Back to the Devices panel |

On the localhost device the Users pane ends with an **`── external ──`
group** (always shown, even when empty): one row per incoming connection
targeting the current user, labelled by the source `user@device`.

### Navigation keys (every pane state)

| Key | Action |
|-----|--------|
| `Enter` | Go one level deeper in the context hierarchy (always offered, listed first) |
| `Esc` | Go one level back (always offered, listed first) |
| `Up/Down/Left/Right` | Walk directionally from the virtual center: overview → Devices (Up/Right) or Logs strip (Down) focused → Users sub-panel (Right) or fullscreen Logs (Down); the opposite key rewinds (see [navigation.md](../navigation.md)) |
| `PgUp/PgDn` | Scroll Logs pane |

### Plain mode (idle prompt)

The plain fallback offers the global keys on its prompt line, built from
the idle question's binding table:
`[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, k=connections, h=help, ?=help]`.

### In Add Device Prompt

| Key | Action |
|-----|--------|
| `y` | Add device |
| `n` or `q` | Skip device |
| `s` | Sync git |

### In Delete Mode

| Key | Action |
|-----|--------|
| `Up/Down` | Select device row in the Devices panel |
| `Enter` | Confirm deletion |
| `Esc` | Cancel |

### In Trust Key Prompt

| Key | Action |
|-----|--------|
| `y` | Trust new signing key |
| `n` | Reject new signing key |

### In Rename Prompt

| Key | Action |
|-----|--------|
| Type new name | Rename device |
| `Enter` | Accept suggestion |
| `Esc` | Skip rename |

## Styling

The UI uses color and styling to convey information:

| Element | Style |
|---------|-------|
| Devices panel | Green border |
| Divider rows between sections | Dark gray |
| Idle status | Base color (green) |
| Info message | Cyan text |
| Success | Green background, black text |
| Warning | Yellow, blinking |
| Security | Red, blinking |
| Selected row | Reversed colors |
| Active panel (several visible) | Double border, theme color; lone full-screen panels keep plain borders |
| Deactivated panel (another one active) | Gray border |
| Beacon pulse | Cyan background (bright) → bold cyan → bold base |

## Help Panel

Press `?` to show the help panel, which displays the help text
(`ops::help::usage_text` — interactive commands only, no CLI subcommands) with
section headers in bold. Use `Up/Down/PgUp/PgDn` to scroll, `Esc` to close.

## Logging

There is no verbose mode: everything the user needs is always logged. All
messages go through `log()` / `log_device()` and land in the Logs strip
(or the fullscreen Logs pane) or on stdout (plain mode). See the [Logging policy](../logging.md)
for the two hard rules: a log line must precede any operation that may prompt
for a password, and every non-standard path must be logged.
