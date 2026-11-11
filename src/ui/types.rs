//! UI types: events, levels, devices, questions, and actions.

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
    MainMenu,
    /// Detected devices panel
    Detected,
    /// Added devices panel
    Added,
    /// Logs pane (formerly Activity)
    Logs,
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
