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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Idle(QuestionDef),
    /// Add the front of the pending queue (plain mode only: the
    /// full-screen ui adds devices from the Detected panel instead).
    AddDevice(QuestionDef, String, String), // name, ip
    TrustKey(QuestionDef, String), // name
    Rename(QuestionDef, String, String), // name, suggested
    Delete(QuestionDef, String), // name
    /// Wipe the whole configuration (key 'w'), with the number of device
    /// blocks that would be removed from ~/.ssh/config.
    Wipe(QuestionDef, usize), // count
    /// Install the 'regroup-cd' shell function (key 'c') into `file`
    /// (~/.zshrc or ~/.bashrc), replacing any existing definition.
    SetupCd(QuestionDef, String), // file
    /// Install SSH key: select target device (key 'i')
    InstallKeySelectDevice(QuestionDef, Vec<String>), // devices
    /// Install SSH key: select target user
    InstallKeySelectUser(QuestionDef, String, Vec<String>), // device, users
    /// Install SSH key: confirm installation
    InstallKeyConfirm(QuestionDef, String, String, String, String), // from_user, from_device, to_user, to_device
    /// Remove SSH key: select connection (key 'r')
    RemoveKeySelectConnection(QuestionDef, Vec<(String, String, String)>), // connections
    /// Remove SSH key: confirm removal
    RemoveKeyConfirm(QuestionDef, String, String, String, String, String), // connection_id, from_user, from_device, to_user, to_device
}

impl Question {
    pub fn def(&self) -> &QuestionDef {
        match self {
            Question::Idle(def) => def,
            Question::AddDevice(def, _, _) => def,
            Question::TrustKey(def, _) => def,
            Question::Rename(def, _, _) => def,
            Question::Delete(def, _) => def,
            Question::Wipe(def, _) => def,
            Question::SetupCd(def, _) => def,
            Question::InstallKeySelectDevice(def, _) => def,
            Question::InstallKeySelectUser(def, _, _) => def,
            Question::InstallKeyConfirm(def, _, _, _, _) => def,
            Question::RemoveKeySelectConnection(def, _) => def,
            Question::RemoveKeyConfirm(def, _, _, _, _, _) => def,
        }
    }
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
    /// Return to Main menu from any pane (key 'q').
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

// Static question definitions
pub static IDLE_QUESTION: QuestionDef = QuestionDef {
    title: "Actions",
    bindings: &[],
};

pub static ADD_DEVICE_QUESTION: QuestionDef = QuestionDef {
    title: "New device",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "add"),
        KeyBinding::new(KeyCode::Char('n'), "n", "skip"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync git"),
        KeyBinding::new(KeyCode::Char('v'), "v", "verbose"),
        KeyBinding::new(KeyCode::Char('q'), "q", "quit"),
    ],
};

pub static TRUST_KEY_QUESTION: QuestionDef = QuestionDef {
    title: "Signing key changed",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "trust"),
        KeyBinding::new(KeyCode::Char('n'), "n", "reject"),
    ],
};

pub static RENAME_QUESTION: QuestionDef = QuestionDef {
    title: "Name already used",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "accept"),
        KeyBinding::new(KeyCode::Char('q'), "q", "skip"),
    ],
};

pub static DELETE_QUESTION: QuestionDef = QuestionDef {
    title: "Delete device",
    bindings: &[
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static WIPE_QUESTION: QuestionDef = QuestionDef {
    title: "Wipe configuration",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static SETUP_CD_QUESTION: QuestionDef = QuestionDef {
    title: "Install regroup-cd",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static INSTALL_KEY_SELECT_DEVICE_QUESTION: QuestionDef = QuestionDef {
    title: "Install SSH key",
    bindings: &[
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static INSTALL_KEY_SELECT_USER_QUESTION: QuestionDef = QuestionDef {
    title: "Install SSH key",
    bindings: &[
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Enter, "Enter", "confirm"),
        KeyBinding::new(KeyCode::Char('q'), "q", "back"),
    ],
};

pub static INSTALL_KEY_CONFIRM_QUESTION: QuestionDef = QuestionDef {
    title: "Install SSH key",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static REMOVE_KEY_SELECT_CONNECTION_QUESTION: QuestionDef = QuestionDef {
    title: "Remove SSH key",
    bindings: &[
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

pub static REMOVE_KEY_CONFIRM_QUESTION: QuestionDef = QuestionDef {
    title: "Remove SSH key",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm"),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel"),
    ],
};

// Convenience constructors for Question
impl Question {
    pub fn idle() -> Self {
        Question::Idle(IDLE_QUESTION)
    }

    pub fn add_device(name: String, ip: String) -> Self {
        Question::AddDevice(ADD_DEVICE_QUESTION, name, ip)
    }

    pub fn trust_key(name: String) -> Self {
        Question::TrustKey(TRUST_KEY_QUESTION, name)
    }

    pub fn rename(name: String, suggested: String) -> Self {
        Question::Rename(RENAME_QUESTION, name, suggested)
    }

    pub fn delete(name: String) -> Self {
        Question::Delete(DELETE_QUESTION, name)
    }

    pub fn wipe(count: usize) -> Self {
        Question::Wipe(WIPE_QUESTION, count)
    }

    pub fn setup_cd(file: String) -> Self {
        Question::SetupCd(SETUP_CD_QUESTION, file)
    }

    pub fn install_key_select_device(devices: Vec<String>) -> Self {
        Question::InstallKeySelectDevice(INSTALL_KEY_SELECT_DEVICE_QUESTION, devices)
    }

    pub fn install_key_select_user(device: String, users: Vec<String>) -> Self {
        Question::InstallKeySelectUser(INSTALL_KEY_SELECT_USER_QUESTION, device, users)
    }

    pub fn install_key_confirm(from_user: String, from_device: String, to_user: String, to_device: String) -> Self {
        Question::InstallKeyConfirm(INSTALL_KEY_CONFIRM_QUESTION, from_user, from_device, to_user, to_device)
    }

    pub fn remove_key_select_connection(connections: Vec<(String, String, String)>) -> Self {
        Question::RemoveKeySelectConnection(REMOVE_KEY_SELECT_CONNECTION_QUESTION, connections)
    }

    pub fn remove_key_confirm(connection_id: String, from_user: String, from_device: String, to_user: String, to_device: String) -> Self {
        Question::RemoveKeyConfirm(REMOVE_KEY_CONFIRM_QUESTION, connection_id, from_user, from_device, to_user, to_device)
    }
}
