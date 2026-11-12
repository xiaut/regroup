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

/// A hint entry for the action bar: one or more equivalent keys that
/// trigger the same action, shown as `[Enter/Right] fullscreen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Key labels joined with `/`, e.g. "Enter/Right".
    pub keys: String,
    /// What the key does, e.g. "fullscreen".
    pub description: &'static str,
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

    /// Check if we can navigate in the specified direction.
    pub fn can_navigate(&self, direction: NavDirection) -> bool {
        use NavDirection::*;
        match direction {
            Left => matches!(self.focus, NavFocus::Detected),
            Right => matches!(self.focus, NavFocus::Added) || matches!(self.focus, NavFocus::Center),
            Down => matches!(self.focus, NavFocus::Center),
            Up => matches!(self.focus, NavFocus::Logs),
        }
    }

    /// Handle a navigation key press, return the new navigation state.
    pub fn handle_key(self, key: KeyCode, has_users: bool) -> Self {
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
            (Added, Some(Fullscreen), Enter) if has_users => Navigation { focus: Users, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Enter) => Navigation { focus: Logs, depth: Some(Fullscreen) },

            // Left arrow: move left
            (Detected, Some(Selected), Left) => Navigation::overview(),
            (Detected, Some(Fullscreen), Left) => Navigation { focus: Detected, depth: Some(Selected) },
            (Added, Some(Selected), Left) => Navigation::overview(),
            (Added, Some(Fullscreen), Left) => Navigation { focus: Added, depth: Some(Selected) },
            (Users, Some(Fullscreen), Left) => Navigation { focus: Added, depth: Some(Fullscreen) },

            // Right arrow: move right
            (Detected, Some(Selected), Right) => Navigation { focus: Detected, depth: Some(Fullscreen) },
            (Added, Some(Selected), Right) => Navigation { focus: Added, depth: Some(Fullscreen) },
            (Added, Some(Fullscreen), Right) if has_users => Navigation { focus: Users, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Right) => Navigation { focus: Logs, depth: Some(Fullscreen) },

            // Down arrow
            (Logs, Some(Selected), Down) => Navigation { focus: Logs, depth: Some(Fullscreen) },

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
            (NavFocus::Users, Some(NavDepth::Fullscreen)) => ActivePane::users(false),
            (NavFocus::Logs, Some(NavDepth::Fullscreen)) => ActivePane::logs(),
            // For Selected depth, return MainMenu to show dashboard with panel highlighted
            _ => ActivePane::main_menu(),
        }
    }

    /// Hints for the action bar, built dynamically from two sources:
    /// navigation keys (arrows/Enter/Esc, from the state machine) and
    /// command keys (character shortcuts, per pane). Entries that share
    /// the same description are merged, e.g. `[Left/d] Detected`.
    pub fn hints(&self, has_users: bool, confirmed_user: bool) -> Vec<Hint> {
        let mut actions: Vec<(&'static str, &'static str)> = Vec::new();
        actions.extend(self.nav_keys(has_users));
        actions.extend(self.command_keys(confirmed_user));
        merge_hints(&actions)
    }

    /// Navigation keys that work in this state (from `handle_key` +
    /// `list_nav_action`). Returns (key_label, description) pairs.
    fn nav_keys(&self, has_users: bool) -> Vec<(&'static str, &'static str)> {
        use NavDepth::*;
        use NavFocus::*;
        match (self.focus, self.depth) {
            (Center, None) => vec![
                ("Left", "Detected"),
                ("Right", "Added"),
                ("Dn", "Logs"),
            ],
            (Detected, Some(Selected)) => vec![
                ("Enter", "fullscreen"),
                ("Right", "fullscreen"),
                ("Esc", "back"),
                ("Left", "back"),
            ],
            (Detected, Some(Fullscreen)) => vec![
                ("Esc", "back"),
                ("Left", "back"),
                ("Up", "select"),
                ("Dn", "select"),
            ],
            (Added, Some(Selected)) => vec![
                ("Enter", "fullscreen"),
                ("Right", "fullscreen"),
                ("Esc", "back"),
                ("Left", "back"),
            ],
            (Added, Some(Fullscreen)) => {
                let mut v = vec![
                    ("Esc", "back"),
                    ("Left", "back"),
                    ("Up", "select"),
                    ("Dn", "select"),
                ];
                if has_users {
                    v.push(("Enter", "users"));
                    v.push(("Right", "users"));
                }
                v
            }
            (Users, Some(Fullscreen)) => vec![
                ("Esc", "machines"),
                ("Left", "machines"),
                ("Up", "select"),
                ("Dn", "select"),
            ],
            (Logs, Some(Selected)) => vec![
                ("Enter", "fullscreen"),
                ("Right", "fullscreen"),
                ("Dn", "fullscreen"),
                ("Esc", "back"),
            ],
            (Logs, Some(Fullscreen)) => vec![
                ("Esc", "back"),
                ("Up", "scroll"),
                ("Dn", "scroll"),
                ("PgUp", "scroll page"),
                ("PgDn", "scroll page"),
            ],
            _ => vec![],
        }
    }

    /// Command keys (character shortcuts) available in this state.
    /// Returns (key_label, description) pairs.
    fn command_keys(&self, confirmed_user: bool) -> Vec<(&'static str, &'static str)> {
        use NavDepth::*;
        use NavFocus::*;
        match (self.focus, self.depth) {
            (Center, None) => vec![
                ("d", "Detected"),
                ("a", "Added"),
                ("l", "Logs"),
                ("?", "help"),
                ("q", "quit"),
            ],
            (Detected, Some(Selected)) => vec![("?", "help")],
            (Detected, Some(Fullscreen)) => vec![
                ("a", "add"),
                ("n", "ignore"),
                ("q", "no pane"),
                ("?", "help"),
            ],
            (Added, Some(Selected)) => vec![("?", "help")],
            (Added, Some(Fullscreen)) => vec![
                ("d", "delete"),
                ("s", "sync"),
                ("w", "wipe"),
                ("c", "setup-cd"),
                ("q", "no pane"),
                ("?", "help"),
            ],
            (Users, Some(Fullscreen)) => {
                let mut v = vec![("s", "sync"), ("q", "no pane"), ("?", "help")];
                if confirmed_user {
                    v.extend([("i", "install key"), ("r", "remove key"), ("a", "add temp user")]);
                } else {
                    v.extend([("a", "add temp user"), ("x", "remove temp user")]);
                }
                v
            }
            (Logs, Some(Selected)) => vec![("?", "help")],
            (Logs, Some(Fullscreen)) => vec![
                ("v", "verbose"),
                ("q", "no pane"),
                ("?", "help"),
            ],
            _ => vec![],
        }
    }
}

/// Merge (key, description) pairs into Hints, joining keys that share
/// the same description: ("Left", "Detected") + ("d", "Detected")
/// becomes Hint { keys: "Left/d", description: "Detected" }.
fn merge_hints(actions: &[(&'static str, &'static str)]) -> Vec<Hint> {
    let mut order: Vec<&'static str> = Vec::new();
    let mut map: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for &(key, desc) in actions {
        if let Some(entry) = map.iter_mut().find(|(d, _)| *d == desc) {
            if !entry.1.contains(&key) {
                entry.1.push(key);
            }
        } else {
            map.push((desc, vec![key]));
            order.push(desc);
        }
    }
    order.iter().map(|desc| {
        let keys = map.iter().find(|(d, _)| d == desc).unwrap().1.join("/");
        Hint { keys, description: desc }
    }).collect()
}

/// Direction for navigation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
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
        KeyBinding::new(KeyCode::Enter, "Enter", "select"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Left, "Left", "Detected"),
        KeyBinding::new(KeyCode::Right, "Right", "Added"),
        KeyBinding::new(KeyCode::Down, "Dn", "Logs"),
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
        KeyBinding::new(KeyCode::Enter, "Enter", "fullscreen"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add"),
        KeyBinding::new(KeyCode::Char('n'), "n", "ignore"),
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

pub static ADDED_PANE: Pane = Pane {
    name: "Added devices",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "fullscreen"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Right, "Right", "users"),
        KeyBinding::new(KeyCode::Char('d'), "d", "delete"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('w'), "w", "wipe"),
        KeyBinding::new(KeyCode::Char('c'), "c", "setup-cd"),
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

/// Users panel with a confirmed user selected: machine+user actions.
pub static USERS_PANE_CONFIRMED: Pane = Pane {
    name: "Users",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "select"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Left, "Left", "machines"),
        KeyBinding::new(KeyCode::Char('i'), "i", "install key"),
        KeyBinding::new(KeyCode::Char('r'), "r", "remove key"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add temp user"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

/// Users panel with a temporary (or no) user selected: only the actions a
/// temporary user allows. Unavailable actions are not shown.
pub static USERS_PANE_TEMPORARY: Pane = Pane {
    name: "Users",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "select"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Up, "Up", "select"),
        KeyBinding::new(KeyCode::Down, "Dn", "select"),
        KeyBinding::new(KeyCode::Left, "Left", "machines"),
        KeyBinding::new(KeyCode::Char('a'), "a", "add temp user"),
        KeyBinding::new(KeyCode::Char('x'), "x", "remove temp user"),
        KeyBinding::new(KeyCode::Char('s'), "s", "sync"),
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
        KeyBinding::new(KeyCode::Char('?'), "?", "help"),
    ],
};

pub static LOGS_PANE: Pane = Pane {
    name: "Logs",
    bindings: &[
        KeyBinding::new(KeyCode::Enter, "Enter", "fullscreen"),
        KeyBinding::new(KeyCode::Esc, "Esc", "back"),
        KeyBinding::new(KeyCode::Up, "Up", "scroll line"),
        KeyBinding::new(KeyCode::Down, "Dn", "scroll line"),
        KeyBinding::new(KeyCode::PageUp, "PgUp", "scroll page"),
        KeyBinding::new(KeyCode::PageDown, "PgDn", "scroll page"),
        KeyBinding::new(KeyCode::Char('v'), "v", "toggle verbose"),
        KeyBinding::new(KeyCode::Char('q'), "q", "no pane"),
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

/// Direction for navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
