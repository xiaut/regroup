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

/// Keys that work while the help panel is open. Esc is the only key
/// that closes it; `?` opens it from any pane (HELP_KEY) but does not
/// close it.
pub static HELP_BINDINGS: &[KeyBinding] = &[
    KeyBinding::internal(KeyCode::Up, "Up", "scroll help"),
    KeyBinding::internal(KeyCode::Down, "Down", "scroll help"),
    KeyBinding::internal(KeyCode::PageUp, "PgUp", "scroll help"),
    KeyBinding::internal(KeyCode::PageDown, "PgDn", "scroll help"),
    KeyBinding::internal(KeyCode::Esc, "Esc", "close"),
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

/// Which section of the Devices pane a row belongs to. The Devices pane
/// is one list, each section under its divider row: localhost (the local
/// device), then added (known) devices, then detected (pending) ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceSection {
    /// The device regroup is running on (always the first row).
    Local,
    /// A device in the configuration (added).
    Added,
    /// A device announced but not yet accepted (detected).
    Detected,
}

impl DeviceSection {
    /// Label of the divider row introducing this section of the
    /// Devices pane.
    pub fn label(&self) -> &'static str {
        match self {
            DeviceSection::Local => "localhost",
            DeviceSection::Added => "added",
            DeviceSection::Detected => "detected",
        }
    }

    /// Idle status text of a row in this section, when it has no
    /// animated message.
    pub fn idle_status(&self) -> &'static str {
        match self {
            DeviceSection::Local => "this device",
            DeviceSection::Added => "known",
            DeviceSection::Detected => "detected",
        }
    }
}

/// New navigation model (docs/navigation.md).
///
/// The user is virtually at the center of the screen.
/// Arrows move directionally: Up (Devices, the single device list on top
/// of the dashboard), Down (Logs, below).
/// Enter goes deeper, Esc goes back.
/// Users sub-panel is nested inside Devices (reached by Right/Enter from
/// Devices with the local or an added device selected).
///
/// Navigation focus: which direction from center.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavFocus {
    /// At center, no panel engaged - all panels visible (overview)
    Center,
    /// Focus on the Devices panel (up from center)
    Devices,
    /// Focus on Users sub-panel (nested inside Devices, right from Devices fullscreen)
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
    ///
    /// `selected_section` is the section of the selected row in the
    /// Devices pane: from the Devices pane, the local row and an added
    /// device row open the Users sub-panel (a detected row has no users
    /// yet); on any other row Right/Enter do nothing.
    pub fn handle_key(self, key: KeyCode, selected_section: Option<DeviceSection>) -> Self {
        use KeyCode::*;
        use NavDepth::*;
        use NavFocus::*;

        match (self.focus, self.depth, key) {
            // From Center (overview) - panels are always navigable, even
            // when empty. The Devices panel is above the center, the Logs
            // panel below: Up/Right and Down point at them.
            (Center, None, Up | Right) => Navigation { focus: Devices, depth: Some(Selected) },
            (Center, None, Down) => Navigation { focus: Logs, depth: Some(Selected) },
            (Center, None, Left) | (Center, None, Enter) | (Center, None, Esc) => self,

            // Esc always goes back. From the Devices/Users panes Left also
            // goes back: Up/Down are consumed by the row lists (the
            // documented exception), so they cannot exit the pane.
            (Devices, Some(Selected), Esc | Left) => Navigation::overview(),
            (Users, Some(Selected), Esc | Left) => Navigation { focus: Devices, depth: Some(Selected) },
            (Logs, Some(Selected), Esc) => Navigation::overview(),
            (Logs, Some(Fullscreen), Esc) => Navigation { focus: Logs, depth: Some(Selected) },

            // Right/Enter open the Users sub-panel when the local or an
            // added device row is selected (a detected row has no users
            // yet). The Devices pane has no fullscreen mode anymore: the
            // Logs pane is a small always-visible strip, so the Devices
            // pane is always big.
            (Devices, Some(Selected), Right | Enter) => match selected_section {
                Some(DeviceSection::Local | DeviceSection::Added) => {
                    Navigation { focus: Users, depth: Some(Selected) }
                }
                _ => self,
            },

            // Logs pane (below center): Down/Enter zoom it fullscreen.
            (Logs, Some(Selected), Down | Enter) => Navigation { focus: Logs, depth: Some(Fullscreen) },
            (Logs, Some(Selected), Up) => Navigation::overview(),

            // Note: in Logs/Fullscreen, Up/Down scroll the content (handled
            // outside handle_key); only Esc exits fullscreen logs.

            // Other cases: no navigation change
            _ => self,
        }
    }

    /// Get list navigation action for Up/Down in lists.
    /// Only returns actions when a list pane is focused (Selected depth).
    pub fn list_nav_action(&self, key: KeyCode) -> Option<Action> {
        use KeyCode::*;

        // List navigation only works when the list pane is focused
        if self.depth != Some(NavDepth::Selected) {
            return None;
        }

        match (self.focus, key) {
            (NavFocus::Devices, Up) => Some(Action::DevicesSelectPrev),
            (NavFocus::Devices, Down) => Some(Action::DevicesSelectNext),
            (NavFocus::Users, Up) => Some(Action::UsersSelectPrev),
            (NavFocus::Users, Down) => Some(Action::UsersSelectNext),
            _ => None,
        }
    }

    /// Get the corresponding ActivePane for this navigation state.
    /// The Devices and Users panes are shown as soon as they are focused
    /// (the Devices pane has no fullscreen mode); the Logs pane only
    /// fills the screen at Fullscreen depth — at Selected depth the
    /// dashboard stays up with the Logs strip highlighted.
    pub fn active_pane(&self) -> ActivePane {
        match (self.focus, self.depth) {
            (NavFocus::Center, _) => ActivePane::main_menu(),
            (NavFocus::Devices, Some(_)) => ActivePane::devices(),
            (NavFocus::Users, Some(_)) => ActivePane::users(),
            (NavFocus::Logs, Some(NavDepth::Fullscreen)) => ActivePane::logs(),
            _ => ActivePane::main_menu(),
        }
    }

}

/// The extra context the key dictionary of a pane state depends on,
/// beyond `(focus, depth)`: which row is selected in the Devices pane
/// (drives the action keys of the fullscreen Devices pane) and whether
/// the selected user in the Users pane is confirmed.
#[derive(Clone, Copy, Debug)]
pub struct PaneKeyCtx {
    pub confirmed_user: bool,
    pub selected_section: Option<DeviceSection>,
    /// Whether the local device is in the shared configuration — drives
    /// the localhost row's keys (add vs remove from the shared config).
    pub local_in_config: bool,
}

/// The key dictionary of a pane state: every key that works in
/// `(focus, depth)`, navigation and command keys alike, in one list.
/// `bindings()` in ui::keys appends the one global key (help) on top.
///
/// This is the single source of truth for the pane states: the hint
/// lines and the key dispatch are both derived from these lists, so a
/// key that is not listed here is not handled.
pub fn pane_keys(focus: NavFocus, depth: Option<NavDepth>, ctx: PaneKeyCtx) -> &'static [KeyBinding] {
    use NavDepth::*;
    use NavFocus::*;
    match (focus, depth) {
        (Center, None) => CENTER_KEYS,
        (Center, Some(Selected)) => &[],
        (Center, Some(Fullscreen)) => &[],
        (Devices, None) => &[],
        // The Devices pane is always big (the Logs pane is a small
        // strip), so focusing it already activates the row list.
        (Devices, Some(Selected)) => match ctx.selected_section {
            Some(DeviceSection::Added) => DEVICES_ADDED_KEYS,
            Some(DeviceSection::Detected) => DEVICES_DETECTED_KEYS,
            // The localhost row offers add or remove from the shared
            // configuration, depending on its membership.
            Some(DeviceSection::Local) => {
                if ctx.local_in_config {
                    DEVICES_LOCAL_ADDED_KEYS
                } else {
                    DEVICES_LOCAL_NOT_ADDED_KEYS
                }
            }
            _ => DEVICES_LOCAL_NOT_ADDED_KEYS,
        },
        (Devices, Some(Fullscreen)) => &[],
        (Users, None) => &[],
        (Users, Some(Selected)) => {
            // On the localhost device no install/remove key: connecting
            // to the current device is nonsense.
            if ctx.selected_section == Some(DeviceSection::Local) {
                if ctx.confirmed_user {
                    USERS_LOCAL_CONFIRMED_KEYS
                } else {
                    USERS_TEMP_KEYS
                }
            } else if ctx.confirmed_user {
                USERS_CONFIRMED_KEYS
            } else {
                USERS_TEMP_KEYS
            }
        }
        (Users, Some(Fullscreen)) => &[],
        (Logs, None) => &[],
        (Logs, Some(Selected)) => LOGS_SELECTED_KEYS,
        (Logs, Some(Fullscreen)) => LOGS_FULLSCREEN_KEYS,
    }
}

use KeyCode::*;

static CENTER_KEYS: &[KeyBinding] = &[
    // The Devices panel is above the center, the Logs panel below.
    KeyBinding::internal(Up, "Up", "devices"),
    KeyBinding::internal(Right, "Right", "devices"),
    KeyBinding::internal(Down, "Down", "logs"),
    // Quit is only offered on the main menu (overview).
    KeyBinding::new(Char('q'), "q", "quit", Effect::Action(Action::Quit)),
];

/// Focused Devices pane, local row selected, local device NOT in the
/// shared configuration: it can be added back (key 'a' — plain "add", the
/// data stays recoverable in git history, unlike a device deletion).
/// The Users sub-panel (Right/Enter) shows the local device's own users.
static DEVICES_LOCAL_NOT_ADDED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Left, "Left", "back"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::internal(Enter, "Enter", "users"),
    KeyBinding::internal(Right, "Right", "users"),
    KeyBinding::new(Char('a'), "a", "add", Effect::Action(Action::AddLocalToConfig)),
];

/// Focused Devices pane, local row selected, local device in the shared
/// configuration: it can be removed from it (key 'd' — plain "remove",
/// not "delete": the local files stay recoverable in git history, and
/// ~/.ssh/config / known_hosts are untouched — the local device has no
/// such traces).
static DEVICES_LOCAL_ADDED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Left, "Left", "back"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::internal(Enter, "Enter", "users"),
    KeyBinding::internal(Right, "Right", "users"),
    KeyBinding::new(Char('d'), "d", "remove", Effect::Action(Action::RemoveLocalFromConfig)),
];

/// Focused Devices pane with an added device selected: device
/// actions plus entry into the Users sub-panel (Right/Enter).
static DEVICES_ADDED_KEYS: &[KeyBinding] = &[
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

/// Focused Devices pane with a detected device selected: only
/// add/ignore make sense on a pending device.
static DEVICES_DETECTED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "back"),
    KeyBinding::internal(Left, "Left", "back"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('a'), "a", "add", Effect::Action(Action::Add)),
    KeyBinding::new(Char('n'), "n", "ignore", Effect::Action(Action::Ignore)),
];

static USERS_CONFIRMED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "devices"),
    KeyBinding::internal(Left, "Left", "devices"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('s'), "s", "sync", Effect::Action(Action::Sync)),
    KeyBinding::new(Char('i'), "i", "install key", Effect::Action(Action::InstallKeyToSelected)),
    KeyBinding::new(Char('r'), "r", "remove key", Effect::Action(Action::RemoveKeyFromSelected)),
    KeyBinding::new(Char('a'), "a", "add temp user", Effect::Action(Action::AddTempUser)),
];

/// Users pane of the localhost device, confirmed user selected: same as
/// USERS_CONFIRMED_KEYS but without install/remove key — connecting to
/// the current device is nonsense.
static USERS_LOCAL_CONFIRMED_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "devices"),
    KeyBinding::internal(Left, "Left", "devices"),
    KeyBinding::internal(Up, "Up", "select"),
    KeyBinding::internal(Down, "Down", "select"),
    KeyBinding::new(Char('s'), "s", "sync", Effect::Action(Action::Sync)),
    KeyBinding::new(Char('a'), "a", "add temp user", Effect::Action(Action::AddTempUser)),
];

static USERS_TEMP_KEYS: &[KeyBinding] = &[
    KeyBinding::internal(Esc, "Esc", "devices"),
    KeyBinding::internal(Left, "Left", "devices"),
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
];

/// Merge (key, description) pairs into Hints, joining keys that share
/// the same description: ("Left", "Detected") + ("d", "Detected")
/// becomes Hint { keys: "Left/d", description: "Detected" }.
/// Which pane is currently active for navigation and actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivePane {
    /// No pane active - in Main menu
    MainMenu(Pane),
    /// The single devices panel: one list with the local device, the
    /// added devices and the detected devices (divider rows between).
    Devices(Pane),
    /// Users sub-panel of the Devices panel: the users of the selected
    /// device (local or added). Device+user actions live here, not in the
    /// device panel.
    Users(Pane),
    /// Logs pane (formerly Activity)
    Logs(Pane),
}

impl ActivePane {
    pub fn name(&self) -> &'static str {
        match self {
            ActivePane::MainMenu(pane) => pane.name,
            ActivePane::Devices(pane) => pane.name,
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

/// One device row of the Devices panel.
#[derive(Clone)]
pub struct UiDevice {
    /// SSH public key: identifies the row for per-device messages.
    pub key: String,
    pub name: String,
    pub ip: Option<String>,
    pub port: u16,
    /// Which section of the Devices pane the row belongs to (local,
    /// added, detected) — decides the divider rows and the action keys.
    pub section: DeviceSection,
    /// Whether the device is in the shared configuration (its
    /// `device.json` is present in the git layer). Only meaningful for
    /// the local row, where it drives the divider label
    /// `── localhost (added|not added) ──` and the add/remove keys;
    /// always true for added rows, false for detected ones.
    pub in_shared_config: bool,
}

/// Status of a user on the selected device, shown in the Users panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserStatus {
    /// Added manually in the Users panel, persisted in the `tmp/` overlay
    /// (never synced): enough to build the git remote URL and sync, until
    /// the device's declared users arrive.
    Temporary,
    /// Declared by the device itself in its configuration (users/).
    Confirmed,
    /// Confirmed, and the local user has an SSH key installed towards
    /// this user (connection file in the local device's connections/).
    KeyInstalled,
}

/// One user of the selected device, for the Users panel.
#[derive(Clone)]
pub struct UiUser {
    pub name: String,
    pub status: UserStatus,
    /// Whether the user is in the shared configuration (its file is
    /// present in the git layer): the Users pane's super-state, shown as
    /// the `── added ──` / `── not added ──` group dividers.
    pub added: bool,
    /// Pinned first row of the Users pane — the current user on the
    /// localhost device, always shown under its own
    /// `── current user (added|not added) ──` divider.
    pub pinned: bool,
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
    /// full-screen ui adds devices from the Devices panel instead).
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
    /// Add the detected device selected in the Devices panel.
    Add,
    /// Ignore the detected device selected in the Devices panel (for this run).
    Ignore,
    /// Add the front of the pending queue (plain mode).
    AddCurrent,
    /// Skip the front of the pending queue (plain mode).
    SkipCurrent,
    TrustKey,
    RejectKey,
    Rename(String),
    SkipRename,
    /// Enter delete mode (device selection in the Devices panel).
    DeleteMode,
    /// Select previous row in the Devices panel.
    DevicesSelectPrev,
    /// Select next row in the Devices panel.
    DevicesSelectNext,
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
    /// Install SSH key to the currently selected device in the Devices panel.
    InstallKeyToSelected,
    /// Remove SSH key from the currently selected device in the Devices panel.
    RemoveKeyFromSelected,
    /// Delete the currently selected added device in the Devices panel.
    DeleteSelected,
    /// Print the CLI help (plain mode; the full-screen ui shows its help
    /// panel instead, toggled by '?').
    Help,
    /// Select previous user in the Users panel.
    UsersSelectPrev,
    /// Select next user in the Users panel.
    UsersSelectNext,
    /// Start typing a temporary user name for the selected device (key 'a').
    AddTempUser,
    /// The typed temporary user name was submitted.
    TempUserName(String),
    /// Cancel typing a temporary user name.
    CancelTempUser,
    /// Remove the selected temporary user (key 'x').
    RemoveTempUser,
    /// Add the local device to the shared configuration (key 'a' on the
    /// localhost row when it is not added): create its device file and
    /// the current user's file in the git layer and commit them.
    AddLocalToConfig,
    /// Remove the local device from the shared configuration (key 'd' on
    /// the localhost row when it is added): delete its directory from the
    /// git layer and commit the deletion. No ~/.ssh/config block or
    /// known_hosts entry is touched — the local device has none.
    RemoveLocalFromConfig,
}

// Static pane definitions. The keys of a pane are not listed here: they
// live in the navigation state's binding table (Navigation::bindings).
pub static MAIN_MENU_PANE: Pane = Pane { name: "Main menu" };

pub static DEVICES_PANE: Pane = Pane { name: "Devices" };

pub static USERS_PANE: Pane = Pane { name: "Users" };

pub static LOGS_PANE: Pane = Pane { name: "Logs" };

// Convenience constructors for ActivePane
impl ActivePane {
    pub const fn main_menu() -> Self {
        ActivePane::MainMenu(MAIN_MENU_PANE)
    }

    pub const fn devices() -> Self {
        ActivePane::Devices(DEVICES_PANE)
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
    fn devices_panel_has_no_fullscreen_mode_anymore() {
        let center = Navigation::overview();
        // Up focuses the Devices panel (it is above the center); the row
        // list is active right away — there is no fullscreen step.
        let focused = center.handle_key(Up, None);
        assert_eq!(focused, nav(NavFocus::Devices, Some(NavDepth::Selected)));
        assert_eq!(focused.handle_key(Up, None), focused);
        assert_eq!(focused.handle_key(Down, None), focused);
        assert_eq!(focused.list_nav_action(Up), Some(Action::DevicesSelectPrev));
        assert_eq!(focused.list_nav_action(Down), Some(Action::DevicesSelectNext));
        // Left/Esc go back to the overview (Up/Down are consumed by the list).
        assert_eq!(focused.handle_key(Left, None), center);
        assert_eq!(focused.handle_key(Esc, None), center);
        // Right also focuses the Devices panel from the overview; Left does not.
        assert_eq!(center.handle_key(Left, None), center);
        assert_eq!(center.handle_key(Right, None), focused);
    }

    #[test]
    fn users_entry_from_local_and_added_rows_only() {
        let focused = nav(NavFocus::Devices, Some(NavDepth::Selected));
        // The local row and an added device row: Right/Enter open the
        // Users sub-panel.
        for section in [Some(DeviceSection::Local), Some(DeviceSection::Added)] {
            for key in [Enter, Right] {
                assert_eq!(
                    focused.handle_key(key, section),
                    nav(NavFocus::Users, Some(NavDepth::Selected))
                );
            }
        }
        // On a detected row (no users yet) or with no selection, Right/Enter do nothing.
        for section in [Some(DeviceSection::Detected), None] {
            assert_eq!(focused.handle_key(Enter, section), focused);
            assert_eq!(focused.handle_key(Right, section), focused);
        }
        let users = nav(NavFocus::Users, Some(NavDepth::Selected));
        assert_eq!(users.handle_key(Left, None), focused);
        assert_eq!(users.handle_key(Esc, None), focused);
        assert_eq!(users.list_nav_action(Down), Some(Action::UsersSelectNext));
    }

    #[test]
    fn logs_down_goes_deeper_up_rewinds() {
        let selected = Navigation::overview().handle_key(Down, None);
        assert_eq!(selected, nav(NavFocus::Logs, Some(NavDepth::Selected)));
        let fullscreen = selected.handle_key(Down, None);
        assert_eq!(fullscreen, nav(NavFocus::Logs, Some(NavDepth::Fullscreen)));
        // Fullscreen logs: Up/Down scroll the content, only Esc exits.
        assert_eq!(fullscreen.handle_key(Up, None), fullscreen);
        assert_eq!(
            fullscreen.handle_key(Esc, None),
            nav(NavFocus::Logs, Some(NavDepth::Selected))
        );
        let back_to_center = selected.handle_key(Up, None);
        assert_eq!(back_to_center, Navigation::overview());
    }

    #[test]
    fn right_does_not_zoom_the_logs_panel() {
        let selected = nav(NavFocus::Logs, Some(NavDepth::Selected));
        assert_eq!(selected.handle_key(Right, None), selected);
    }

    #[test]
    fn list_navigation_only_when_a_list_pane_is_focused() {
        // At the overview, no list is active.
        assert_eq!(Navigation::overview().list_nav_action(Up), None);
        // A focused list pane navigates with Up/Down at any depth.
        let focused = nav(NavFocus::Devices, Some(NavDepth::Selected));
        assert_eq!(focused.list_nav_action(Up), Some(Action::DevicesSelectPrev));
        assert_eq!(focused.list_nav_action(Down), Some(Action::DevicesSelectNext));
        // The Logs pane is not a row list.
        let logs = nav(NavFocus::Logs, Some(NavDepth::Fullscreen));
        assert_eq!(logs.list_nav_action(Up), None);
    }
}
