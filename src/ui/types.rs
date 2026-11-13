//! UI types: events, levels, devices, questions, and actions.

use crossterm::event::KeyCode;

/// What a bound key does when pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The key produces this action for the interactive loop.
    Action(Action),
    /// The key submits the text being typed (rename, temporary user).
    SubmitInput,
    /// The key is handled internally by the ui (navigation, scrolling,
    /// help panel, text capture): it produces no action.
    Internal,
}

/// A key binding: the physical key, how it displays, and what it does.
///
/// This is the single source of truth for key handling: the hint lines
/// are built from these tables, and a key press is resolved by looking
/// it up in the same table. A key that is not listed is not handled,
/// so a hint can never advertise a key that does nothing, and a key can
/// never work without being advertised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: KeyCode,
    pub label: &'static str,
    pub description: &'static str,
    pub effect: Effect,
}

impl KeyBinding {
    pub const fn new(key: KeyCode, label: &'static str, description: &'static str, effect: Effect) -> Self {
        Self { key, label, description, effect }
    }

    pub const fn internal(key: KeyCode, label: &'static str, description: &'static str) -> Self {
        Self { key, label, description, effect: Effect::Internal }
    }
}

/// The one key that works in every pane state: help. `bindings()` in
/// ui::keys appends it to every pane's key list.
pub const HELP_KEY: KeyBinding = KeyBinding::internal(KeyCode::Char('?'), "?", "help");

/// Keys that work while the help panel is open.
pub static HELP_BINDINGS: &[KeyBinding] = &[
    KeyBinding::internal(KeyCode::Up, "Up", "scroll help"),
    KeyBinding::internal(KeyCode::Down, "Down", "scroll help"),
    KeyBinding::internal(KeyCode::PageUp, "PgUp", "scroll help"),
    KeyBinding::internal(KeyCode::PageDown, "PgDn", "scroll help"),
    KeyBinding::internal(KeyCode::Esc, "Esc", "close"),
    KeyBinding::internal(KeyCode::Char('?'), "?", "close"),
    KeyBinding::internal(KeyCode::Char('q'), "q", "close"),
];

/// Render a binding list as a hint line body: `[key] desc   [key] desc`.
pub fn bindings_hint_text(bindings: &[KeyBinding]) -> String {
    bindings
        .iter()
        .map(|b| format!("[{}] {}", b.label, b.description))
        .collect::<Vec<_>>()
        .join("   ")
}

/// A hint entry for the action bar: one or more equivalent keys that
/// trigger the same action, shown as `[Enter/Right] fullscreen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Key labels joined with `/`, e.g. "Enter/Right".
    pub keys: String,
    /// What the key does, e.g. "fullscreen".
    pub description: &'static str,
}

/// Pane definition: name. The keys of a pane come from the navigation
/// state's binding table, not from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    pub name: &'static str,
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

/// New navigation model (docs/navigation.md).
///
/// The user is virtually at the center of the screen.
/// Arrows move directionally: Left (Detected), Right (Added), Down (Logs).
/// Enter goes deeper, Esc goes back.
/// Users sub-panel is nested inside Added (reached by Right from Added fullscreen).
///
/// Navigation focus: which direction from center.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavFocus {
    /// At center, no panel engaged - all panels visible (overview)
    Center,
    /// Focus on Detected panel (left from center)
    Detected,
    /// Focus on Added panel (right from center)
    Added,
    /// Focus on Users sub-panel (nested inside Added, right from Added fullscreen)
    Users,
    /// Focus on Logs panel (down from center)
    Logs,
}

/// Navigation depth: how engaged we are with the focused panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavDepth {
    /// Panel is selected/highlighted (first step from center)
    Selected,
    /// Panel is fullscreen (second step from center)
    Fullscreen,
}

/// Complete navigation state: focus + depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Navigation {
    pub focus: NavFocus,
    pub depth: Option<NavDepth>,
}

impl Default for NavFocus {
    fn default() -> Self {
        NavFocus::Center
    }
}

impl Navigation {
    /// Create overview state (center, no panel engaged).
    pub const fn overview() -> Self {
        Self {
            focus: NavFocus::Center,
            depth: None,
        }
    }

    /// Check if this is the overview state.
    pub fn is_overview(&self) -> bool {
        self.focus == NavFocus::Center && self.depth.is_none()
    }

    /// Check if a panel is at Selected depth.
    pub fn is_selected(&self) -> bool {
        self.depth == Some(NavDepth::Selected)
    }

    /// Check if a panel is at Fullscreen depth.
    pub fn is_fullscreen(&self) -> bool {
        self.depth == Some(NavDepth::Fullscreen)
    }

    /// Check if we're in the Users sub-panel.
    pub fn in_users(&self) -> bool {
        self.focus == NavFocus::Users
    }

    /// Check if we're in the Logs panel.
    pub fn in_logs(&self) -> bool {
        self.focus == NavFocus::Logs
    }

    /// Handle a navigation key press, return the new navigation state.
    pub fn handle_key(self, key: KeyCode, _has_users: bool) -> Self {
        use KeyCode::*;
        use NavDepth::*;
        use NavFocus::*;

        match (self.focus, self.depth, key) {
            // From Center (overview) - panels are always navigable, even when empty
            (Center, None, Left) => Navigation { focus: Detected, depth: Some(Selected) },
            (Center, None, Right) => Navigation { focus: Added, depth: Some(Selected) },
            (Center, None, Down) => Navigation { focus: Logs, depth: Some(Selected) },
            (Center, None, Up) => self,
            (Center, None, Enter) => self, // Needs direction first
            (Center, None, Esc) => self,

            // Esc: always go back
            (Detected, Some(Selected), Esc) => Navigation::overview(),
            (Detected, Some(Fullscreen), Esc) => Navigation { focus: Detected, depth: Some(Selected) },
            (Added, Some(Selected), Esc) => Navigation::overview(),
            (Added, Some(Fullscreen), Esc) => Navigation { focus: Added, depth: Some(Selected) },
            (Users, Some(Fullscreen), Esc) => Navigation { focus: Added, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Esc) => Navigation::overview(),
            (Logs, Some(Fullscreen), Esc) => Navigation { focus: Logs, depth: Some(Selected) },

            // Enter: go one level deeper
            (Detected, Some(Selected), Enter) => Navigation { focus: Detected, depth: Some(Fullscreen) },
            (Added, Some(Selected), Enter) => Navigation { focus: Added, depth: Some(Fullscreen) },
            (Added, Some(Fullscreen), Enter) => Navigation { focus: Users, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Enter) => Navigation { focus: Logs, depth: Some(Fullscreen) },

            // Left arrow: toward the Detected panel (left of center),
            // each press goes one step deeper; Right rewinds.
            (Detected, Some(Selected), Left) => Navigation { focus: Detected, depth: Some(Fullscreen) },
            (Detected, Some(Selected), Right) => Navigation::overview(),
            (Detected, Some(Fullscreen), Right) => Navigation { focus: Detected, depth: Some(Selected) },
            (Added, Some(Selected), Left) => Navigation::overview(),
            (Added, Some(Fullscreen), Left) => Navigation { focus: Added, depth: Some(Selected) },
            (Users, Some(Fullscreen), Left) => Navigation { focus: Added, depth: Some(Fullscreen) },

            // Right arrow: toward the Added panel (right of center),
            // each press goes one step deeper; Left rewinds.
            (Added, Some(Selected), Right) => Navigation { focus: Added, depth: Some(Fullscreen) },
            (Added, Some(Fullscreen), Right) => Navigation { focus: Users, depth: Some(Fullscreen) },

            // Down/Up arrows: toward the Logs panel (below center)
            (Logs, Some(Selected), Down) => Navigation { focus: Logs, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Up) => Navigation::overview(),

            // Note: in Logs/Fullscreen, Up/Down scroll the content (handled
            // outside handle_key); only Esc exits fullscreen logs.

            // Other cases: no navigation change
            _ => self,
        }
    }

    /// Get list navigation action for Up/Down in lists.
    /// Only returns actions when in Fullscreen depth (not Selected).
    pub fn list_nav_action(&self, key: KeyCode) -> Option<Action> {
        use KeyCode::*;
        
        // List navigation only works in Fullscreen depth
        if !self.is_fullscreen() {
            return None;
        }

        match (self.focus, key) {
            (NavFocus::Detected, Up) => Some(Action::PendingSelectPrev),
            (NavFocus::Detected, Down) => Some(Action::PendingSelectNext),
            (NavFocus::Added, Up) => Some(Action::AddedSelectPrev),
            (NavFocus::Added, Down) => Some(Action::AddedSelectNext),
            (NavFocus::Users, Up) => Some(Action::UsersSelectPrev),
            (NavFocus::Users, Down) => Some(Action::UsersSelectNext),
            _ => None,
        }
    }

    /// Get the corresponding ActivePane for this navigation state.
    /// For Selected depth, returns MainMenu to show the dashboard with the panel highlighted.
    /// For Fullscreen depth, returns the specific pane to show it full-screen.
    pub fn active_pane(&self) -> ActivePane {
        match (self.focus, self.depth) {
            (NavFocus::Center, _) => ActivePane::main_menu(),
            (NavFocus::Detected, Some(NavDepth::Fullscreen)) => ActivePane::detected(),
            (NavFocus::Added, Some(NavDepth::Fullscreen)) => ActivePane::added(),
            (NavFocus::Users, Some(NavDepth::Fullscreen)) => ActivePane::users(),
            (NavFocus::Logs, Some(NavDepth::Fullscreen)) => ActivePane::logs(),
            // For Selected depth, return MainMenu to show dashboard with panel highlighted
            _ => ActivePane::main_menu(),
        }
    }

}

/// The key dictionary of a pane state: every key that works in
/// `(focus, depth)`, navigation and command keys alike, in one list.
/// `bindings()` in ui::keys appends the one global key (help) on top.
///
/// This is the single source of truth for the pane states: the hint
/// lines and the key dispatch are both derived from these lists, so a
/// key that is not listed here is not handled.
pub fn pane_keys(focus: NavFocus, depth: Option<NavDepth>, confirmed_user: bool) -> &'static [KeyBinding] {
    use NavDepth::*;
    use NavFocus::*;
    match (focus, depth) {
        (Center, None) => CENTER_KEYS,
        (Center, Some(Selected)) => &[],
        (Center, Some(Fullscreen)) => &[],
        (Detected, None) => &[],
        (Detected, Some(Selected)) => DETECTED_SELECTED_KEYS,
        (Detected, Some(Fullscreen)) => DETECTED_FULLSCREEN_KEYS,
        (Added, None) => &[],
        (Added, Some(Selected)) => ADDED_SELECTED_KEYS,
        (Added, Some(Fullscreen)) => ADDED_FULLSCREEN_KEYS,
        (Users, None) => &[],
        (Users, Some(Selected)) => &[],
        (Users, Some(Fullscreen)) => {
            if confirmed_user {
                USERS_FULLSCREEN_CONFIRMED_KEYS
            } else {
                USERS_FULLSCREEN_TEMP_KEYS
            }
        }
        (Logs, None) => &[],
        (Logs, Some(Selected)) => LOGS_SELECTED_KEYS,
        (Logs, Some(Fullscreen)) => LOGS_FULLSCREEN_KEYS,
    }
}

use KeyCode::*;

static CENTER_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Left, "Left", "Detected"),
    KeyBinding::internal(Right, "Right", "Added"),
    KeyBinding::internal(Down, "Down", "Logs"),
    // Quit is only offered on the main menu (overview).
    KeyBinding::new(Char('q'), "q", "quit", Effect::Action(Action::Quit)),
];

static DETECTED_SELECTED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Enter, "Enter", "fullscreen"),
    KeyBinding::internal(Left, "Left", "fullscreen"),
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Right, "Right", "back"),
];

static DETECTED_FULLSCREEN_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Right, "Right", "back"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('a'), "a", "add", Effect::Action(Action::Add)),
    KeyBinding::new(Char('n'), "n", "ignore", Effect::Action(Action::Ignore)),
];

static ADDED_SELECTED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Enter, "Enter", "fullscreen"),
    KeyBinding::internal(Right, "Right", "fullscreen"),
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Left, "Left", "back"),
];

static ADDED_FULLSCREEN_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Left, "Left", "back"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::internal(Enter, "Enter", "users"),
    KeyBinding::internal(Right, "Right", "users"),
    KeyBinding::new(Char('d'), "d", "delete", Effect::Action(Action::DeleteSelected)),
    KeyBinding::new(Char('s'), "s", "sync", Effect::Action(Action::Sync)),
    KeyBinding::new(Char('w'), "w", "wipe", Effect::Action(Action::WipeMode)),
    KeyBinding::new(Char('c'), "c", "setup-cd", Effect::Action(Action::SetupCdMode)),
];

static USERS_FULLSCREEN_CONFIRMED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "machines"),
    KeyBinding::internal(Left, "Left", "machines"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('s'), "s", "sync", Effect::Action(Action::Sync)),
    KeyBinding::new(Char('i'), "i", "install key", Effect::Action(Action::InstallKeyToSelected)),
    KeyBinding::new(Char('r'), "r", "remove key", Effect::Action(Action::RemoveKeyFromSelected)),
    KeyBinding::new(Char('a'), "a", "add temp user", Effect::Action(Action::AddTempUser)),
];

static USERS_FULLSCREEN_TEMP_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "machines"),
    KeyBinding::internal(Left, "Left", "machines"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('s'), "s", "sync", Effect::Action(Action::Sync)),
    KeyBinding::new(Char('a'), "a", "add temp user", Effect::Action(Action::AddTempUser)),
    KeyBinding::new(Char('x'), "x", "remove temp user", Effect::Action(Action::RemoveTempUser)),
];

static LOGS_SELECTED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Enter, "Enter", "fullscreen"),
    KeyBinding::internal(Down, "Down", "fullscreen"),
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Up, "Up", "back"),
];

static LOGS_FULLSCREEN_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Up, "Up", "scroll"),
    KeyBinding::internal(Down, "Down", "scroll"),
    KeyBinding::internal(PageUp, "PgUp", "scroll page"),
    KeyBinding::internal(PageDown, "PgDn", "scroll page"),
    KeyBinding::new(Char('v'), "v", "verbose", Effect::Action(Action::ToggleVerbose)),
];

/// Merge (key, description) pairs into Hints, joining keys that share
/// the same description: ("Left", "Detected") + ("d", "Detected")
/// becomes Hint { keys: "Left/d", description: "Detected" }.
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Print the CLI help (plain mode; the full-screen ui shows its help
    /// panel instead, toggled by '?').
    Help,
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

// Static pane definitions. The keys of a pane are not listed here: they
// live in the navigation state's binding table (Navigation::bindings).
pub static MAIN_MENU_PANE: Pane = Pane { name: "Main menu" };

pub static DETECTED_PANE: Pane = Pane { name: "Detected devices" };

pub static ADDED_PANE: Pane = Pane { name: "Added devices" };

pub static USERS_PANE: Pane = Pane { name: "Users" };

pub static LOGS_PANE: Pane = Pane { name: "Logs" };

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

    pub const fn users() -> Self {
        ActivePane::Users(USERS_PANE)
    }

    pub const fn logs() -> Self {
        ActivePane::Logs(LOGS_PANE)
    }
}

// Static question definitions. Each binding carries the effect of its
// key; the ui dispatches key presses by looking them up here (plus
// GLOBAL_BINDINGS), exactly the list the hint line is built from.

/// Idle commands, used by the plain ui prompt and dispatch (the
/// full-screen ui builds its Idle keys from the navigation table).
pub static IDLE_QUESTION: QuestionDef = QuestionDef {
    title: "Actions",
    bindings: &[
        KeyBinding::new(KeyCode::Char('q'), "q", "quit", Effect::Action(Action::Quit)),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync git", Effect::Action(Action::Sync)),
        KeyBinding::new(KeyCode::Char('d'), "d", "delete", Effect::Action(Action::DeleteMode)),
        KeyBinding::new(KeyCode::Char('w'), "w", "wipe", Effect::Action(Action::WipeMode)),
        KeyBinding::new(KeyCode::Char('c'), "c", "setup-cd", Effect::Action(Action::SetupCdMode)),
        KeyBinding::new(KeyCode::Char('k'), "k", "connections", Effect::Action(Action::ConnectionsMode)),
        KeyBinding::new(KeyCode::Char('v'), "v", "verbose", Effect::Action(Action::ToggleVerbose)),
        KeyBinding::new(KeyCode::Char('h'), "h", "help", Effect::Action(Action::Help)),
        KeyBinding::new(KeyCode::Char('?'), "?", "help", Effect::Action(Action::Help)),
    ],
};

pub static ADD_DEVICE_QUESTION: QuestionDef = QuestionDef {
    title: "New device",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "add", Effect::Action(Action::AddCurrent)),
        KeyBinding::new(KeyCode::Char('n'), "n", "skip", Effect::Action(Action::SkipCurrent)),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync git", Effect::Action(Action::Sync)),
        KeyBinding::new(KeyCode::Char('d'), "d", "delete", Effect::Action(Action::DeleteMode)),
        KeyBinding::new(KeyCode::Char('w'), "w", "wipe", Effect::Action(Action::WipeMode)),
        KeyBinding::new(KeyCode::Char('c'), "c", "setup-cd", Effect::Action(Action::SetupCdMode)),
        KeyBinding::new(KeyCode::Char('k'), "k", "connections", Effect::Action(Action::ConnectionsMode)),
        KeyBinding::new(KeyCode::Char('v'), "v", "verbose", Effect::Action(Action::ToggleVerbose)),
        KeyBinding::new(KeyCode::Char('?'), "?", "help", Effect::Action(Action::Help)),
    ],
};

pub static TRUST_KEY_QUESTION: QuestionDef = QuestionDef {
    title: "Signing key changed",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "trust", Effect::Action(Action::TrustKey)),
        KeyBinding::new(KeyCode::Char('n'), "n", "reject", Effect::Action(Action::RejectKey)),
    ],
};

pub static ADD_TEMP_USER_QUESTION: QuestionDef = QuestionDef {
    title: "Add temporary user",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "confirm", Effect::SubmitInput),
        KeyBinding::new(KeyCode::Esc, "Esc", "cancel", Effect::Action(Action::CancelTempUser)),
    ],
};

/// While typing a new name, every character key is captured as text;
/// only Enter and Esc are bound.
pub static RENAME_QUESTION: QuestionDef = QuestionDef {
    title: "Name already used",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "accept", Effect::SubmitInput),
        KeyBinding::new(KeyCode::Esc, "Esc", "skip", Effect::Action(Action::SkipRename)),
    ],
};

pub static DELETE_QUESTION: QuestionDef = QuestionDef {
    title: "Delete device",
    bindings: &[
        KeyBinding::new(KeyCode::Up, "Up", "select", Effect::Action(Action::SelectPrev)),
        KeyBinding::new(KeyCode::Down, "Down", "select", Effect::Action(Action::SelectNext)),
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm", Effect::Action(Action::DeleteConfirm)),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel", Effect::Action(Action::DeleteCancel)),
    ],
};

pub static WIPE_QUESTION: QuestionDef = QuestionDef {
    title: "Wipe configuration",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm", Effect::Action(Action::WipeConfirm)),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel", Effect::Action(Action::WipeCancel)),
    ],
};

pub static SETUP_CD_QUESTION: QuestionDef = QuestionDef {
    title: "Install regroup-cd",
    bindings: &[
        KeyBinding::new(KeyCode::Char('y'), "y", "confirm", Effect::Action(Action::SetupCdConfirm)),
        KeyBinding::new(KeyCode::Char('n'), "n", "cancel", Effect::Action(Action::SetupCdCancel)),
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


#[cfg(test)]
mod navigation_tests {
    use super::*;
    use crossterm::event::KeyCode::*;

    fn nav(focus: NavFocus, depth: Option<NavDepth>) -> Navigation {
        Navigation { focus, depth }
    }

    #[test]
    fn detected_panel_left_goes_deeper_right_rewinds() {
        let center = Navigation::overview();
        let selected = center.handle_key(Left, false);
        assert_eq!(selected, nav(NavFocus::Detected, Some(NavDepth::Selected)));
        let fullscreen = selected.handle_key(Left, false);
        assert_eq!(fullscreen, nav(NavFocus::Detected, Some(NavDepth::Fullscreen)));
        let back_to_selected = fullscreen.handle_key(Right, false);
        assert_eq!(back_to_selected, nav(NavFocus::Detected, Some(NavDepth::Selected)));
        let back_to_center = back_to_selected.handle_key(Right, false);
        assert_eq!(back_to_center, Navigation::overview());
    }

    #[test]
    fn added_panel_right_goes_deeper_left_rewinds() {
        let selected = Navigation::overview().handle_key(Right, false);
        assert_eq!(selected, nav(NavFocus::Added, Some(NavDepth::Selected)));
        let fullscreen = selected.handle_key(Right, false);
        assert_eq!(fullscreen, nav(NavFocus::Added, Some(NavDepth::Fullscreen)));
        let back_to_selected = fullscreen.handle_key(Left, false);
        assert_eq!(back_to_selected, nav(NavFocus::Added, Some(NavDepth::Selected)));
        let back_to_center = back_to_selected.handle_key(Left, false);
        assert_eq!(back_to_center, Navigation::overview());
    }

    #[test]
    fn users_entry_is_never_skipped_even_with_no_users() {
        let fullscreen = nav(NavFocus::Added, Some(NavDepth::Fullscreen));
        assert_eq!(
            fullscreen.handle_key(Right, false),
            nav(NavFocus::Users, Some(NavDepth::Fullscreen))
        );
        assert_eq!(
            fullscreen.handle_key(Enter, false),
            nav(NavFocus::Users, Some(NavDepth::Fullscreen))
        );
        let users = nav(NavFocus::Users, Some(NavDepth::Fullscreen));
        assert_eq!(
            users.handle_key(Left, false),
            nav(NavFocus::Added, Some(NavDepth::Fullscreen))
        );
        assert_eq!(
            users.handle_key(Esc, false),
            nav(NavFocus::Added, Some(NavDepth::Fullscreen))
        );
    }

    #[test]
    fn logs_down_goes_deeper_up_rewinds() {
        let selected = Navigation::overview().handle_key(Down, false);
        assert_eq!(selected, nav(NavFocus::Logs, Some(NavDepth::Selected)));
        let fullscreen = selected.handle_key(Down, false);
        assert_eq!(fullscreen, nav(NavFocus::Logs, Some(NavDepth::Fullscreen)));
        // Fullscreen logs: Up/Down scroll the content, only Esc exits.
        assert_eq!(fullscreen.handle_key(Up, false), fullscreen);
        assert_eq!(
            fullscreen.handle_key(Esc, false),
            nav(NavFocus::Logs, Some(NavDepth::Selected))
        );
        let back_to_center = selected.handle_key(Up, false);
        assert_eq!(back_to_center, Navigation::overview());
    }

    #[test]
    fn right_does_not_zoom_the_logs_panel() {
        let selected = nav(NavFocus::Logs, Some(NavDepth::Selected));
        assert_eq!(selected.handle_key(Right, false), selected);
    }

    #[test]
    fn list_navigation_only_in_fullscreen() {
        let selected = nav(NavFocus::Added, Some(NavDepth::Selected));
        assert_eq!(selected.list_nav_action(Up), None);
        let fullscreen = nav(NavFocus::Added, Some(NavDepth::Fullscreen));
        assert_eq!(fullscreen.list_nav_action(Up), Some(Action::AddedSelectPrev));
        assert_eq!(fullscreen.list_nav_action(Down), Some(Action::AddedSelectNext));
        let logs = nav(NavFocus::Logs, Some(NavDepth::Fullscreen));
        assert_eq!(logs.list_nav_action(Up), None);
    }
}
