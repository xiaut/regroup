# ui Module

The `ui` module provides the terminal user interface for interactive mode.

## Overview

This module implements two UI backends with the same interface:

1. **Full-screen TUI** (ratatui): A rich terminal interface with device panels, Logs panel, and question display
2. **Plain fallback**: A simple line-based interface for non-terminal output (piped to files or other commands)

The module automatically selects the appropriate backend based on whether stdout is a terminal.

## Module Structure

```
ui/
├── mod.rs          # Module root with public re-exports
├── types.rs        # Public types: UiEvent, Level, Device, Question, Action
├── logging.rs      # Logging functions: log, log_device, pulse_device, restore
├── interface.rs    # Ui enum (Tty/Plain) with unified interface
├── tty.rs          # TtyUi: full-screen ratatui implementation
├── plain.rs        # PlainUi: line-based fallback implementation
└── render.rs       # Rendering functions and styling utilities
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
    AddDevice { name: String, ip: String },
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
    Add,            // Add selected device
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
┌─────────────────────────────────────────┐
│  [Detected]        │  [Added]           │
│  ┌─────────────────┐ ┌─────────────────┐ │
│  │ Name  │ IP      │ │ Name  │ IP      │ │
│  │-----  │ ------ │ │-----  │ ------ │ │
│  │ dev1  │ 1.2.3.4│ │ dev2  │ 5.6.7.8│ │
│  └─────────────────┘ └─────────────────┘ │
├─────────────────────────────────────────┤
│  [Logs]                              │
│  Device dev1 detected                    │
│  Device dev2 IP updated                  │
├─────────────────────────────────────────┤
│  [Status]                                │
│  2 device(s) detected — add them?       │
│  [Up/Down] select   [Enter/y] add   [n]    │
│  ignore   [s] sync   [d] delete   [?]    │
│  help   [q] quit                         │
└─────────────────────────────────────────┘
```

### Panels

- **Detected panel** (left, yellow): Devices that have announced but not been accepted
- **Added panel** (right, green): Devices already in the configuration.
  A **Users sub-panel** is nested inside this panel for the selected device;
  it lists the device's users with their status (temporary — to confirm /
  confirmed / SSH key installed). It is a sub-context of the device, not a
  device, not a sibling panel: continuing with `Right` past the Added panel
  (selected, then fullscreen) enters the sub-panel (first user selected),
  and `Left` returns to the device level. Device+user actions
  (install/remove SSH key) live there and only appear for a confirmed user;
  a temporary user only allows the git sync that brings the device's
  declared users. See [navigation.md](../navigation.md).
- **Logs pane** (middle): General messages and logs
- **Question block** (bottom): Current question and available actions

### Narrow Screen Layout

On screens narrower than 80 characters, panels stack vertically:
```
┌─────────────────────────────────────────┐
│  [Detected]                              │
│  ┌─────────────────────────────────────┐│
│  │ Name  │ IP      │ Port │ Status     ││
│  └─────────────────────────────────────┘│
├─────────────────────────────────────────┤
│  [Added]                                 │
│  ┌─────────────────────────────────────┐│
│  │ Name  │ IP      │ Port │ Status     ││
│  └─────────────────────────────────────┘│
├─────────────────────────────────────────┤
│  [Logs]                              │
├─────────────────────────────────────────┤
│  [Status]                                │
└─────────────────────────────────────────┘
```

## Plain UI

For non-terminal output (piped), the plain UI provides a line-based interface:

```
[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, h=help]: 
Add device1 (192.168.1.100) to config? [y/N/q/s]: y
Add device2 (192.168.1.101) to config? [y/N/q/s]: n
[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, h=help]: q
```

## Key Bindings

### Global Keys (Idle)

| Key | Action |
|-----|--------|
| `q` | Quit |
| `s` | Sync git (pull/push) |
| `d` | Delete mode (select device to delete) |
| `w` | Wipe configuration |
| `c` | Setup regroup-cd shell function |
| `k` | Connections management |
| `i` | Install SSH key |
| `r` | Remove SSH key |
| `?` | Toggle help panel |
| `Enter` | Go one level deeper in the context hierarchy (always offered, listed first) |
| `Esc` | Go one level back (always offered, listed first; only exit from the fullscreen Logs panel) |
| `Up/Down/Left/Right` | Walk directionally from the virtual center: overview → panel selected → fullscreen → sub-panel; opposite key rewinds (see [navigation.md](../navigation.md)) |
| `PgUp/PgDn` | Scroll Logs pane |

### In Detected Panel

| Key | Action |
|-----|--------|
| `Enter` or `y` | Add selected device |
| `n` | Ignore selected device |

### In Add Device Prompt

| Key | Action |
|-----|--------|
| `y` | Add device |
| `n` or `q` | Skip device |
| `s` | Sync git |

### In Delete Mode

| Key | Action |
|-----|--------|
| `Up/Down` | Select device in Added panel |
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
| Detected panel | Yellow border |
| Added panel | Green border |
| Idle status | Base color (cyan/blue) |
| Info message | Cyan text |
| Success | Green background, black text |
| Warning | Yellow, blinking |
| Security | Red, blinking |
| Selected row | Reversed colors |
| Active panel (several visible) | Double border, theme color; lone full-screen panels keep plain borders |
| Deactivated panel (another one active) | Gray border |
| Beacon pulse | Cyan background (bright) → bold cyan → bold base |

## Help Panel

Press `?` to show the help panel, which displays the CLI usage text with section headers in bold. Use `Up/Down/PgUp/PgDn` to scroll, `?` or `Esc` to close.

## Logging

There is no verbose mode: everything the user needs is always logged. All
messages go through `log()` / `log_device()` and land in the Logs pane
(full-screen) or on stdout (plain mode). See the [Logging policy](../logging.md)
for the two hard rules: a log line must precede any operation that may prompt
for a password, and every non-standard path must be logged.
