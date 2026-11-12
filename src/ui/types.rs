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
    /// Users sub-panel of the Added panel: the users of the selected
    /// machine. Machine+user actions live here, not in the machine panel.
    Users(Pane),
    /// Logs pane (formerly Activity)
    Logs(Pane),
}

impl ActivePane {
    pub fn name(&self) -> &'static str {
        match self {
            ActivePane::MainMenu(pane) => pane.name,
            ActivePane::Detected(pane) => pane.name,
            ActivePane::Added(pane) => pane.name,
            ActivePane::Users(pane) => pane.name,
            ActivePane::Logs(pane) => pane.name,
        }
    }

    pub fn bindings(&self) -> &'static [KeyBinding] {
        match self {
            ActivePane::MainMenu(pane) => pane.bindings,
            ActivePane::Detected(pane) => pane.bindings,
            ActivePane::Added(pane) => pane.bindings,
            ActivePane::Users(pane) => pane.bindings,
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

/// Status of a user on the selected machine, shown in the Users panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserStatus {
    /// Added manually this run, in memory only: enough to build the git
    /// remote URL and sync, until the machine's declared users arrive.
    Temporary,
    /// Declared by the machine itself in its configuration (users/).
    Confirmed,
    /// Confirmed, and the local user has an SSH key installed towards
    /// this user (connection file in the local device's connections/).
    KeyInstalled,
}

/// One user of the selected machine, for the Users panel.
#[derive(Clone)]
pub struct UiUser {
    pub name: String,
    pub status: UserStatus,
}

impl UiUser {
    /// Row label of the status column.
    pub fn status_label(&self) -> &'static str {
        match self.status {
            UserStatus::Temporary => "temporary — to confirm",
            UserStatus::Confirmed => "confirmed",
            UserStatus::KeyInstalled => "SSH key installed",
        }
    }
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
    /// Type the name of a temporary user for `device` (Users panel, key 'a').
    /// The input is handled like the rename question.
    AddTempUser(QuestionDef, String), // device
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
            Question::AddTempUser(def, _) => def,
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
    /// Switch from the Added panel to the Users sub-panel (key Right).
    SwitchToUsers,
    /// Switch back from the Users sub-panel to the Added panel (key Left).
    BackToAdded,
    /// Select previous user in the Users panel.
    UsersSelectPrev,
    /// Select next user in the Users panel.
    UsersSelectNext,
    /// Start typing a temporary user name for the selected machine (key 'a').
    AddTempUser,
    /// The typed temporary user name was submitted.
    TempUserName(String),
    /// Cancel typing a temporary user name.
    CancelTempUser,
    /// Remove the selected temporary user (key 'x').
    RemoveTempUser,
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
        KeyBinding::new(KeyCode::Right, "Right", "users"),
        KeyBinding::new(KeyCode::Char('d'), "d", "delete"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('w'), "w", "wipe"),
        KeyBinding::new(KeyCode::Char('c'), "c", "setup-cd"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

/// Users panel with a confirmed user selected: machine+user actions.
pub static USERS_PANE_CONFIRMED: Pane = Pane {
    name: "Users",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Left, "Left", "machines"),
        KeyBinding::new(KeyCode::Char('i'), "i", "install key"),
        KeyBinding::new(KeyCode::Char('r'), "r", "remove key"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add temp user"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

/// Users panel with a temporary (or no) user selected: only the actions a
/// temporary user allows. Unavailable actions are not shown.
pub static USERS_PANE_TEMPORARY: Pane = Pane {
    name: "Users",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Left, "Left", "machines"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add temp user"),
        KeyBinding::new(KeyCode::Char('x'), "x", "remove temp user"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
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

    /// The Users panel, with the bindings that match the selected user:
    /// a confirmed user unlocks the machine+user actions, a temporary (or
    /// no) user only shows what a temporary user can do.
    pub fn users(confirmed_selected: bool) -> Self {
        if confirmed_selected {
            ActivePane::Users(USERS_PANE_CONFIRMED)
        } else {
            ActivePane::Users(USERS_PANE_TEMPORARY)
        }
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

pub static ADD_TEMP_USER_QUESTION: QuestionDef = QuestionDef {
    title: "Add temporary user",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "confirm"),
        KeyBinding::new(KeyCode::Esc, "Esc", "cancel"),
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

    pub fn add_temp_user(device: String) -> Self {
        Question::AddTempUser(ADD_TEMP_USER_QUESTION, device)
    }
}
