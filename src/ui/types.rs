//! UI types: events, levels, devices, questions, and actions.

use crossterm::event::KeyCode;

/// A key binding: the physical key, how it displays, and what it does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: KeyCode,
    pub label: &'static str,
    pub description: &'static str,
}

impl KeyBinding {
    pub const fn new(key: KeyCode, label: &'static str, description: &'static str) -> Self {
        Self { key, label, description }
    }
}

/// Pane definition: name and its key bindings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    pub name: &'static str,
    pub bindings: &'static [KeyBinding],
}

/// Question definition: title and its key bindings
#[derive(Clone, Copy, Debug)]
pub struct QuestionDef {
    pub title: &'static str,
    pub bindings: &'static [KeyBinding],
}

/// A message for the ui, buffered through the log channel.
pub enum UiEvent {
    /// General message: activity pane (full-screen ui) or stdout.
    Global(String),
    /// UiDevice-specific message: animated status on the device's row.
    UiDevice {
        /// SSH public key: identifies the device's row.
        key: String,
        name: String,
        level: Level,
        /// Short status shown on the row (the row already shows the name).
        msg: String,
        /// Long form for plain mode, and for security messages mirrored
        /// into the activity pane.
        detail: Option<String>,
    },
    /// Beacon received from a device: animate its row even when the
    /// beacon changes nothing (known device, same IP). No text output.
    Pulse {
        /// SSH public key: identifies the device's row.
        key: String,
    },
}

/// Which pane is currently active for navigation and actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivePane {
    /// No pane active - in Main menu
    MainMenu(Pane),
    /// Detected devices panel
    Detected(Pane),
    /// Added devices panel
    Added(Pane),
    /// Logs pane (formerly Activity)
    Logs(Pane),
}

impl ActivePane {
    pub fn name(&self) -> &'static str {
        match self {
            ActivePane::MainMenu(pane) => pane.name,
            ActivePane::Detected(pane) => pane.name,
            ActivePane::Added(pane) => pane.name,
            ActivePane::Logs(pane) => pane.name,
        }
    }

    pub fn bindings(&self) -> &'static [KeyBinding] {
        match self {
            ActivePane::MainMenu(pane) => pane.bindings,
            ActivePane::Detected(pane) => pane.bindings,
            ActivePane::Added(pane) => pane.bindings,
            ActivePane::Logs(pane) => pane.bindings,
        }
    }
}

/// How important a device-specific message is; drives the row animation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Cyan, steady: something happened, no action needed.
    Info,
    /// Green background, steady: the device's state improved.
    Success,
    /// Yellow, blinking: attention needed (question block says more).
    Warning,
    /// Red, blinking: possible spoofing/MITM; also mirrored to Activity.
    Security,
}

/// One device for the device panels.
#[derive(Clone)]
pub struct UiDevice {
    /// SSH public key: identifies the row for per-device messages.
    pub key: String,
    pub name: String,
    pub ip: Option<String>,
    pub port: u16,
}

/// What the interactive loop is asking right now. The ui renders it as the
/// bottom block and derives the active keys from it.
#[derive(Clone, PartialEq)]
pub enum Question {
    /// Nothing pending: only global keys.
    Idle,
    /// Add the front of the pending queue (plain mode only: the
    /// full-screen ui adds devices from the Detected panel instead).
    AddDevice {
        name: String,
        ip: String,
    },
    TrustKey {
        name: String,
    },
    Rename {
        name: String,
        suggested: String,
    },
    Delete {
        name: String,
    },
    /// Wipe the whole configuration (key 'w'), with the number of device
    /// blocks that would be removed from ~/.ssh/config.
    Wipe {
        count: usize,
    },
    /// Install the 'regroup-cd' shell function (key 'c') into `file`
    /// (~/.zshrc or ~/.bashrc), replacing any existing definition.
    SetupCd {
        file: String,
    },
    /// Install SSH key: select target device (key 'i')
    InstallKeySelectDevice {
        devices: Vec<String>,
    },
    /// Install SSH key: select target user
    InstallKeySelectUser {
        device: String,
        users: Vec<String>,
    },
    /// Install SSH key: confirm installation
    InstallKeyConfirm {
        from_user: String,
        from_device: String,
        to_user: String,
        to_device: String,
    },
    /// Remove SSH key: select connection (key 'r')
    RemoveKeySelectConnection {
        connections: Vec<(String, String, String)>,
    },
    /// Remove SSH key: confirm removal
    RemoveKeyConfirm {
        connection_id: String,
        from_user: String,
        from_device: String,
        to_user: String,
        to_device: String,
    },
}

/// A user decision, translated by the ui from key presses (or, in plain
/// mode, from input lines).
pub enum Action {
    Quit,
    Sync,
    /// Add the device selected in the Detected panel.
    Add,
    /// Ignore the device selected in the Detected panel (for this run).
    Ignore,
    /// Add the front of the pending queue (plain mode).
    AddCurrent,
    /// Skip the front of the pending queue (plain mode).
    SkipCurrent,
    TrustKey,
    RejectKey,
    Rename(String),
    SkipRename,
    /// Enter delete mode (device selection in the Added panel).
    DeleteMode,
    /// Select previous in Detected panel (Detected pane navigation).
    PendingSelectPrev,
    /// Select next in Detected panel (Detected pane navigation).
    PendingSelectNext,
    /// Select previous in Added panel (Added pane navigation).
    AddedSelectPrev,
    /// Select next in Added panel (Added pane navigation).
    AddedSelectNext,
    SelectPrev,
    SelectNext,
    DeleteConfirm,
    DeleteCancel,
    /// Enter wipe confirmation (key 'w').
    WipeMode,
    WipeConfirm,
    WipeCancel,
    /// Enter setup-cd confirmation (key 'c').
    SetupCdMode,
    SetupCdConfirm,
    SetupCdCancel,
    /// Enter connections management mode (key 'k').
    ConnectionsMode,
    /// Enter install key mode (key 'i').
    InstallKeyMode,
    /// Confirm install key action.
    InstallKeyConfirm,
    /// Cancel install key action.
    InstallKeyCancel,
    /// Enter remove key mode (key 'r').
    RemoveKeyMode,
    /// Confirm remove key action.
    RemoveKeyConfirm,
    /// Cancel remove key action.
    RemoveKeyCancel,
    /// Toggle verbose mode on/off (key 'v').
    ToggleVerbose,
    /// Install SSH key to the currently selected device in Added pane.
    InstallKeyToSelected,
    /// Remove SSH key from the currently selected device in Added pane.
    RemoveKeyFromSelected,
    /// Delete the currently selected device in Added pane.
    DeleteSelected,
    /// Return to Main menu from any pane (key 'Esc').
    BackToMainMenu,
    /// Switch to Detected pane (key 'd').
    SwitchToDetected,
    /// Switch to Added pane (key 'a').
    SwitchToAdded,
    /// Switch to Logs pane (key 'l').
    SwitchToLogs,
    /// Print the CLI help (plain mode; the full-screen ui shows its help
    /// panel instead, toggled by '?').
    Help,
}

// Static pane definitions
pub static MAIN_MENU_PANE: Pane = Pane {
    name: "Main menu",
    bindings: &[
        KeyBinding::new(KeyCode::Char('d'), "d", "Detected"),
        KeyBinding::new(KeyCode::Char('a'), "a", "Added"),
        KeyBinding::new(KeyCode::Char('l'), "l", "Logs"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
        KeyBinding::new(KeyCode::Char('q'), "q", "quit"),
    ],
};

pub static DETECTED_PANE: Pane = Pane {
    name: "Detected devices",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add"),
        KeyBinding::new(KeyCode::Char('n'), "n", "ignore"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

pub static ADDED_PANE: Pane = Pane {
    name: "Added devices",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('i'), "i", "install key"),
        KeyBinding::new(KeyCode::Char('r'), "r", "remove key"),
        KeyBinding::new(KeyCode::Char('d'), "d", "delete"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('w'), "w", "wipe"),
        KeyBinding::new(KeyCode::Char('c'), "c", "setup-cd"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

pub static LOGS_PANE: Pane = Pane {
    name: "Logs",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Up, "Up", "scroll line"),
        KeyBinding::new(KeyCode::Down, "Dn", "scroll line"),
        KeyBinding::new(KeyCode::PageUp, "PgUp", "scroll page"),
        KeyBinding::new(KeyCode::PageDown, "PgDn", "scroll page"),
        KeyBinding::new(KeyCode::Char('v'), "v", "toggle verbose"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

// Convenience constructors for ActivePane
impl ActivePane {
    pub const fn main_menu() -> Self {
        ActivePane::MainMenu(MAIN_MENU_PANE)
    }

    pub const fn detected() -> Self {
        ActivePane::Detected(DETECTED_PANE)
    }

    pub const fn added() -> Self {
        ActivePane::Added(ADDED_PANE)
    }

    pub const fn logs() -> Self {
        ActivePane::Logs(LOGS_PANE)
    }
}
