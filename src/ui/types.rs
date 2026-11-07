//! UI types: events, levels, devices, questions, and actions.

/// A message for the ui, buffered through the log channel.
pub enum UiEvent {
    /// General message: activity pane (full-screen ui) or stdout.
    Global(String),
    /// Machine-specific message: animated status on the machine's row.
    Device {
        /// SSH public key: identifies the machine's row.
        key: String,
        name: String,
        level: Level,
        /// Short status shown on the row (the row already shows the name).
        msg: String,
        /// Long form for plain mode, and for security messages mirrored
        /// into the activity pane.
        detail: Option<String>,
    },
    /// Beacon received from a machine: animate its row even when the
    /// beacon changes nothing (known machine, same IP). No text output.
    Pulse {
        /// SSH public key: identifies the machine's row.
        key: String,
    },
}

/// Which pane is currently active for navigation and actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivePane {
    /// Detected devices panel (left/upper)
    Detected,
    /// Added devices panel (right/lower)
    Added,
}

/// How important a machine-specific message is; drives the row animation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Cyan, steady: something happened, no action needed.
    Info,
    /// Green background, steady: the machine's state improved.
    Success,
    /// Yellow, blinking: attention needed (question block says more).
    Warning,
    /// Red, blinking: possible spoofing/MITM; also mirrored to Activity.
    Security,
}

/// One machine for the machine panels.
#[derive(Clone)]
pub struct Device {
    /// SSH public key: identifies the row for per-machine messages.
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
    /// full-screen ui adds machines from the Detected panel instead).
    AddDevice { name: String, ip: String },
    TrustKey { name: String },
    Rename { name: String, suggested: String },
    Delete { name: String },
    /// Wipe the whole configuration (key 'w'), with the number of device
    /// blocks that would be removed from ~/.ssh/config.
    Wipe { count: usize },
    /// Install the 'regroup-cd' shell function (key 'c') into `file`
    /// (~/.zshrc or ~/.bashrc), replacing any existing definition.
    SetupCd { file: String },
    /// Install SSH key: select target machine (key 'i')
    InstallKeySelectMachine { machines: Vec<String> },
    /// Install SSH key: select target user
    InstallKeySelectUser { machine: String, users: Vec<String> },
    /// Install SSH key: confirm installation
    InstallKeyConfirm {
        from_user: String,
        from_machine: String,
        to_user: String,
        to_machine: String,
    },
    /// Remove SSH key: select connection (key 'r')
    RemoveKeySelectConnection { connections: Vec<(String, String, String)> },
    /// Remove SSH key: confirm removal
    RemoveKeyConfirm {
        connection_id: String,
        from_user: String,
        from_machine: String,
        to_user: String,
        to_machine: String,
    },
}

/// A user decision, translated by the ui from key presses (or, in plain
/// mode, from input lines).
pub enum Action {
    Quit,
    Sync,
    /// Add the machine selected in the Detected panel.
    Add,
    /// Ignore the machine selected in the Detected panel (for this run).
    Ignore,
    /// Add the front of the pending queue (plain mode).
    AddCurrent,
    /// Skip the front of the pending queue (plain mode).
    SkipCurrent,
    TrustKey,
    RejectKey,
    Rename(String),
    SkipRename,
    /// Enter delete mode (machine selection in the Added panel).
    DeleteMode,
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
    /// Print the CLI help (plain mode; the full-screen ui shows its help
    /// panel instead, toggled by '?').
    Help,
}
