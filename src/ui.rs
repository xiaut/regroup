//! Terminal user interface for interactive mode.
//!
//! Two backends behind the same interface:
//! - a full-screen ratatui interface when stdout is a terminal: two machine
//!   panels (detected machines on the left, added machines on the right when
//!   the screen is wide enough; otherwise detected on top, added below),
//!   an activity log for general messages, and the current question as the
//!   bottom block — the question can never be pushed away by notifications;
//!   key '?' replaces the machine panels and the activity pane with the CLI
//!   help, keeping the bottom block;
//! - a plain line-based fallback otherwise (piped output): prompts are
//!   printed as they change and answers are read as input lines.
//!
//! All modules display messages through [`log`] (general messages, shown in
//! the activity pane) or [`log_device`] (machine-specific messages, shown
//! as a temporary animated status on the machine's row). Before the
//! full-screen ui runs (startup, plain mode) both go to stdout; afterwards
//! they are buffered in a channel.

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};

use crate::cli::usage_text;
use crate::ssh::keys::SshPublicKey;

/// Maximum number of activity messages kept for display.
const LOG_CAP: usize = 1000;

/// How long a machine's row keeps its animated status.
const FLASH_MS: u64 = 2500;
/// Security flashes last longer: the user must not miss them.
const SECURITY_FLASH_MS: u64 = 5000;
/// Half-period of the blink of warning and security flashes.
const BLINK_MS: u64 = 250;
/// How long the pressed key stays highlighted in the hint line.
const KEY_FLASH_MS: u64 = 700;
/// How long a machine's row animates after a beacon is received from it.
const PULSE_MS: u64 = 900;
/// Minimum terminal width to display machine panels side-by-side.
/// Below this, panels stack vertically (detected on top, added below).
const MIN_WIDTH_FOR_SIDE_BY_SIDE: u16 = 80;

/// Sender side of the log channel; unset until [`Ui::start`] creates it.
static LOG_TX: OnceLock<mpsc::Sender<UiEvent>> = OnceLock::new();

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

/// Display a general message: in the activity pane once the full-screen ui
/// runs, on stdout otherwise.
pub fn log(msg: &str) {
    match LOG_TX.get() {
        Some(tx) => {
            let _ = tx.send(UiEvent::Global(msg.to_string()));
        }
        None => println!("{msg}"),
    }
}

/// Display a machine-specific message: animated status on the machine's
/// row in the full-screen ui, `detail` (or `name: msg`) on stdout otherwise.
/// Security messages are additionally kept in the activity pane.
pub fn log_device(
    key: &SshPublicKey,
    name: &str,
    level: Level,
    msg: &str,
    detail: Option<&str>,
) {
    match LOG_TX.get() {
        Some(tx) => {
            let _ = tx.send(UiEvent::Device {
                key: key.as_str().to_string(),
                name: name.to_string(),
                level,
                msg: msg.to_string(),
                detail: detail.map(str::to_string),
            });
        }
        None => match detail {
            Some(detail) => println!("{detail}"),
            None => println!("{name}: {msg}"),
        },
    }
}

/// Pulse a machine's row: the row lights up and fades out for a short
/// while, showing the user that a beacon was received from it (even a
/// silent one: known machine, unchanged IP). Full-screen ui only.
pub fn pulse_device(key: &SshPublicKey) {
    if let Some(tx) = LOG_TX.get() {
        let _ = tx.send(UiEvent::Pulse {
            key: key.as_str().to_string(),
        });
    }
}

/// Best-effort terminal restoration (raw mode off, leave alternate screen),
/// e.g. before `process::exit` from a code path that cannot run destructors.
pub fn restore() {
    ratatui::restore();
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
    InstallKeyConfirm { from_user: String, from_machine: String, to_user: String, to_machine: String },
    /// Remove SSH key: select connection (key 'r')
    RemoveKeySelectConnection { connections: Vec<(String, String, String)> },
    /// Remove SSH key: confirm removal
    RemoveKeyConfirm { connection_id: String, from_user: String, from_machine: String, to_user: String, to_machine: String },
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
    /// Print the CLI help (plain mode; the full-screen ui shows its help
    /// panel instead, toggled by '?').
    Help,
}

/// The interface used by the interactive loop.
pub enum Ui {
    Tty(Box<TtyUi>),
    Plain(PlainUi),
}

impl Ui {
    /// Start the interface: full-screen when stdout is a terminal, plain
    /// otherwise.
    pub fn start() -> Ui {
        if io::stdout().is_terminal() {
            let terminal = ratatui::init();
            let (tx, log_rx) = mpsc::channel();
            let _ = LOG_TX.set(tx);
            Ui::Tty(Box::new(TtyUi {
                terminal,
                log_rx,
                log: Vec::new(),
                scroll: 0,
                question: Question::Idle,
                input: String::new(),
                detected: Vec::new(),
                added: Vec::new(),
                selected: None,
                delete_selected: None,
                show_help: false,
                help_scroll: 0,
                verbose: false,
                flashes: HashMap::new(),
                pulses: HashMap::new(),
                key_flash: None,
            }))
        } else {
            Ui::Plain(PlainUi::new())
        }
    }

    /// Set the current question. The full-screen ui clears the typed input
    /// and resets the activity scroll whenever the question changes.
    pub fn set_question(&mut self, question: Question) {
        match self {
            Ui::Tty(ui) => ui.set_question(question),
            Ui::Plain(ui) => ui.set_question(question),
        }
    }

    /// Update the machine panels (ignored in plain mode).
    pub fn set_devices(&mut self, detected: Vec<Device>, added: Vec<Device>) {
        if let Ui::Tty(ui) = self {
            ui.detected = detected;
            ui.added = added;
        }
    }

    /// Highlight the selected row of the Detected panel.
    pub fn set_selected(&mut self, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.selected = selected;
        }
    }

    /// Highlight the selected row of the Added panel (delete mode).
    pub fn set_delete_selected(&mut self, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.delete_selected = selected;
        }
    }

    /// Show the current verbose state: the activity pane title and the
    /// status hints mention it (ignored in plain mode, where toggling is
    /// reported through the log).
    pub fn set_verbose(&mut self, verbose: bool) {
        if let Ui::Tty(ui) = self {
            ui.verbose = verbose;
        }
    }

    /// Wait up to `timeout` for a user decision.
    pub fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        match self {
            Ui::Tty(ui) => ui.poll_action(timeout),
            Ui::Plain(ui) => ui.poll_action(timeout),
        }
    }

    /// Redraw the display (no-op in plain mode).
    pub fn draw(&mut self) {
        if let Ui::Tty(ui) = self {
            let _ = ui.draw();
        }
    }
}

/// Full-screen ratatui interface.
pub struct TtyUi {
    terminal: DefaultTerminal,
    log_rx: Receiver<UiEvent>,
    log: Vec<String>,
    /// Lines scrolled up from the bottom of the activity pane; 0 follows it.
    scroll: u16,
    question: Question,
    /// Text typed for the rename question.
    input: String,
    /// Machines announced but not accepted yet (Detected panel).
    detected: Vec<Device>,
    /// Machines in the configuration (Added panel).
    added: Vec<Device>,
    /// Row of the Detected panel highlighted by the selection.
    selected: Option<usize>,
    /// Row of the Added panel highlighted in delete mode.
    delete_selected: Option<usize>,
    /// Whether the help panel is shown (key '?'): it replaces the machine
    /// panels and the activity pane; the question block stays interactive.
    show_help: bool,
    /// Lines scrolled up from the bottom of the help panel; 0 follows it.
    help_scroll: u16,
    /// Verbose mode: reported in the activity pane title and the status
    /// hints. The interactive loop keeps it in sync with its own state.
    verbose: bool,
    /// Animated per-machine statuses, by SSH public key.
    flashes: HashMap<String, RowFlash>,
    /// Rows currently pulsing after a received beacon, by SSH public key.
    pulses: HashMap<String, Instant>,
    /// Last key press, briefly highlighted in the hint line.
    key_flash: Option<KeyFlash>,
}

/// A machine's animated status, shown in the last column of its row.
struct RowFlash {
    level: Level,
    msg: String,
    start: Instant,
}

/// A key press, identified by how it appears in the hint line's `[...]`
/// tokens (`s`, `Enter`, `Dn`, ...).
struct KeyFlash {
    key: String,
    start: Instant,
}

/// What the question block needs to render, besides the question itself.
struct QuestionCtx<'a> {
    /// Text typed for the rename question.
    input: &'a str,
    /// Machines waiting for a decision (drives the Idle hints).
    pending: usize,
    /// Whether the help panel is shown.
    help: bool,
    /// Verbose mode: mentioned in the hints.
    verbose: bool,
    /// Last key press, highlighted in the hint line while recent.
    key_flash: Option<&'a KeyFlash>,
}

impl Drop for TtyUi {
    fn drop(&mut self) {
        ratatui::restore();
        // Messages logged since the last draw are printed to stdout so
        // nothing is lost when the ui closes.
        for event in self.log_rx.try_iter() {
            match event {
                UiEvent::Global(msg) => println!("{msg}"),
                UiEvent::Device {
                    name,
                    msg,
                    detail,
                    ..
                } => println!("{}", detail.unwrap_or(format!("{name}: {msg}"))),
                UiEvent::Pulse { .. } => {}
            }
        }
    }
}

impl TtyUi {
    fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            self.input.clear();
            self.scroll = 0;
            // An urgent question takes precedence over the help panel:
            // close it so the question gets the full screen back.
            if self.question != Question::Idle {
                self.show_help = false;
                self.help_scroll = 0;
            }
        }
    }

    /// Move messages logged since the last draw into the pane, capped.
    /// General messages become activity lines; machine-specific messages
    /// become row flashes (security messages are additionally mirrored into
    /// the activity pane so they do not vanish with the animation).
    fn drain_log(&mut self) {
        while let Ok(event) = self.log_rx.try_recv() {
            match event {
                UiEvent::Global(msg) => {
                    for line in msg.split('\n') {
                        self.log.push(line.to_string());
                    }
                }
                UiEvent::Device {
                    key,
                    level,
                    msg,
                    detail,
                    ..
                } => {
                    if level == Level::Security {
                        let text = detail.as_deref().unwrap_or(&msg);
                        for line in text.split('\n') {
                            self.log.push(line.to_string());
                        }
                    }
                    self.flashes.insert(
                        key,
                        RowFlash {
                            level,
                            msg,
                            start: Instant::now(),
                        },
                    );
                }
                UiEvent::Pulse { key } => {
                    self.pulses.insert(key, Instant::now());
                }
            }
        }
        let excess = self.log.len().saturating_sub(LOG_CAP);
        if excess > 0 {
            self.log.drain(..excess);
        }
        // Keep only recent pulses of machines that are still displayed.
        let now = Instant::now();
        self.pulses.retain(|key, t| {
            (self.detected.iter().any(|d| d.key == *key)
                || self.added.iter().any(|d| d.key == *key))
                && now.duration_since(*t) < Duration::from_millis(PULSE_MS)
        });
        // Keep only recent flashes of machines that are still displayed.
        self.flashes.retain(|key, f| {
            (self.detected.iter().any(|d| d.key == *key)
                || self.added.iter().any(|d| d.key == *key))
                && now.duration_since(f.start) < Duration::from_secs(60)
        });
    }

    fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        if !event::poll(timeout).unwrap_or(false) {
            return None;
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => self.key_action(key),
            _ => None,
        }
    }

    /// Translate one key press into an action. Scroll keys are handled here
    /// and do not produce actions. Up/Down select a machine in the active
    /// panel (Detected, or Added in delete mode) instead of scrolling.
    /// Every press is recorded so the hint line can highlight the key.
    fn key_action(&mut self, key: KeyEvent) -> Option<Action> {
        if let Some(label) = key_label(key.code) {
            self.key_flash = Some(KeyFlash {
                key: label,
                start: Instant::now(),
            });
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            return Some(Action::Quit);
        }
        let in_delete = matches!(self.question, Question::Delete { .. });
        let in_wipe = matches!(self.question, Question::Wipe { .. });
        let in_setup = matches!(self.question, Question::SetupCd { .. });
        let typing = matches!(self.question, Question::Rename { .. });
        // Machines can be selected (and added) only when the Detected
        // panel has content and no other question has the keyboard.
        let selectable = self.question == Question::Idle && !self.detected.is_empty();
        match key.code {
            KeyCode::Enter if in_wipe => Some(Action::WipeConfirm),
            KeyCode::Esc if in_wipe => Some(Action::WipeCancel),
            KeyCode::Char('y') if in_wipe => Some(Action::WipeConfirm),
            // In wipe mode, any other command key cancels.
            KeyCode::Char(c) if in_wipe => {
                (c == 'w' || c == 'q').then_some(Action::WipeCancel)
            }
            KeyCode::Enter if in_setup => Some(Action::SetupCdConfirm),
            KeyCode::Esc if in_setup => Some(Action::SetupCdCancel),
            KeyCode::Char('y') if in_setup => Some(Action::SetupCdConfirm),
            // In setup-cd mode, any other command key cancels.
            KeyCode::Char(c) if in_setup => {
                (c == 'c' || c == 'q').then_some(Action::SetupCdCancel)
            }
            KeyCode::Up if in_delete => Some(Action::SelectPrev),
            KeyCode::Down if in_delete => Some(Action::SelectNext),
            KeyCode::Enter if in_delete => Some(Action::DeleteConfirm),
            KeyCode::Esc if in_delete => Some(Action::DeleteCancel),
            // In delete mode, any other command key cancels.
            KeyCode::Char(c) if in_delete => {
                (c == 'd' || c == 'q').then_some(Action::DeleteCancel)
            }
            // While the help panel is shown, the arrow keys scroll it
            // (the machine panels and the activity pane are hidden).
            KeyCode::Up if self.show_help => {
                self.help_scroll = self.help_scroll.saturating_add(1);
                None
            }
            KeyCode::Down if self.show_help => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
                None
            }
            KeyCode::PageUp if self.show_help => {
                self.help_scroll = self.help_scroll.saturating_add(10);
                None
            }
            KeyCode::PageDown if self.show_help => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
                None
            }
            KeyCode::Up if selectable => Some(Action::SelectPrev),
            KeyCode::Down if selectable => Some(Action::SelectNext),
            KeyCode::Enter if selectable => Some(Action::Add),
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_add(1);
                None
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_sub(1);
                None
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
                None
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
                None
            }
            KeyCode::Char(c) => match (&self.question, c) {
                (Question::Idle, 'q') => Some(Action::Quit),
                (Question::Idle, 's') => Some(Action::Sync),
                (Question::Idle, 'd') => Some(Action::DeleteMode),
                (Question::Idle, 'w') => Some(Action::WipeMode),
                (Question::Idle, 'c') => Some(Action::SetupCdMode),
                (Question::Idle, 'k') => Some(Action::ConnectionsMode),
                (Question::Idle, 'i') => Some(Action::InstallKeyMode),
                (Question::Idle, 'r') => Some(Action::RemoveKeyMode),
                (Question::Idle, 'v') => Some(Action::ToggleVerbose),
                (Question::Idle, '?') => {
                    self.show_help = !self.show_help;
                    self.help_scroll = 0;
                    None
                }
                (Question::Idle, 'y') if selectable => Some(Action::Add),
                (Question::Idle, 'n') if selectable => Some(Action::Ignore),
                (Question::AddDevice { .. }, 'y') => Some(Action::AddCurrent),
                (Question::AddDevice { .. }, 'n') => Some(Action::SkipCurrent),
                (Question::AddDevice { .. }, 's') => Some(Action::Sync),
                (Question::AddDevice { .. }, 'd') => Some(Action::DeleteMode),
                (Question::AddDevice { .. }, 'w') => Some(Action::WipeMode),
                (Question::AddDevice { .. }, 'c') => Some(Action::SetupCdMode),
                (Question::AddDevice { .. }, 'k') => Some(Action::ConnectionsMode),
                (Question::AddDevice { .. }, 'i') => Some(Action::InstallKeyMode),
                (Question::AddDevice { .. }, 'r') => Some(Action::RemoveKeyMode),
                (Question::AddDevice { .. }, 'v') => Some(Action::ToggleVerbose),
                (Question::AddDevice { .. }, 'q') => Some(Action::Quit),
                (Question::TrustKey { .. }, 'y') => Some(Action::TrustKey),
                (Question::TrustKey { .. }, 'n') => Some(Action::RejectKey),
                (Question::Rename { .. }, c) => {
                    self.input.push(c);
                    None
                }
                _ => None,
            },
            KeyCode::Esc => match &self.question {
                Question::TrustKey { .. } => Some(Action::RejectKey),
                Question::Rename { .. } => Some(Action::SkipRename),
                _ if self.show_help => {
                    self.show_help = false;
                    self.help_scroll = 0;
                    None
                }
                _ if selectable => Some(Action::Ignore),
                _ => None,
            },
            KeyCode::Backspace if typing => {
                self.input.pop();
                None
            }
            KeyCode::Enter if typing => {
                let name = std::mem::take(&mut self.input);
                Some(Action::Rename(name))
            }
            _ => None,
        }
    }

    fn draw(&mut self) -> io::Result<()> {
        self.drain_log();
        // Split borrows: the closure renders from the data while `draw`
        // mutably borrows the terminal.
        let Self {
            terminal,
            log,
            scroll,
            question,
            input,
            detected,
            added,
            selected,
            delete_selected,
            show_help,
            help_scroll,
            verbose,
            flashes,
            pulses,
            key_flash,
            ..
        } = self;
        let mut cursor = None;
        terminal.draw(|frame| {
            let [machines_area, log_area, question_area] = Layout::vertical([
                Constraint::Percentage(35),
                Constraint::Min(1),
                Constraint::Length(4),
            ])
            .areas(frame.area());
            if *show_help {
                // The help panel replaces the machine panels and the
                // activity pane; the question block stays interactive.
                let help_area = Rect {
                    x: machines_area.x,
                    y: machines_area.y,
                    width: machines_area.width,
                    height: machines_area.height + log_area.height,
                };
                let visible = help_area.height.saturating_sub(2) as usize;
                let max_scroll =
                    usage_text().lines().count().saturating_sub(visible) as u16;
                *help_scroll = (*help_scroll).min(max_scroll);
                render_help(frame, help_area, *help_scroll);
            } else {
                let narrow = frame.area().width < MIN_WIDTH_FOR_SIDE_BY_SIDE;
                if narrow {
                    // Narrow screen: stack machine panels vertically
                    // (detected on top, added below)
                    let [detected_area, added_area] = Layout::vertical([
                        Constraint::Percentage(50),
                        Constraint::Percentage(50),
                    ])
                    .areas(machines_area);
                    render_machines(
                        frame,
                        detected_area,
                        PanelTheme::new("Detected", Color::Yellow, "detected"),
                        detected,
                        *selected,
                        flashes,
                        pulses,
                    );
                    render_machines(
                        frame,
                        added_area,
                        PanelTheme::new("Added", Color::Green, "known"),
                        added,
                        *delete_selected,
                        flashes,
                        pulses,
                    );
                } else {
                    // Wide screen: side-by-side panels
                    let [detected_area, _gap, added_area] = Layout::horizontal([
                        Constraint::Fill(1),
                        Constraint::Length(1),
                        Constraint::Fill(1),
                    ])
                    .areas(machines_area);
                    render_machines(
                        frame,
                        detected_area,
                        PanelTheme::new("Detected", Color::Yellow, "detected"),
                        detected,
                        *selected,
                        flashes,
                        pulses,
                    );
                    render_machines(
                        frame,
                        added_area,
                        PanelTheme::new("Added", Color::Green, "known"),
                        added,
                        *delete_selected,
                        flashes,
                        pulses,
                    );
                }
                render_log(frame, log_area, log, *scroll, *verbose);
            }
            cursor = render_question(
                frame,
                question_area,
                question,
                &QuestionCtx {
                    input,
                    pending: detected.len(),
                    help: *show_help,
                    verbose: *verbose,
                    key_flash: key_flash.as_ref(),
                },
            );
        })?;
        // Show the text cursor only while typing a name.
        if let Some(position) = cursor {
            let _ = terminal.set_cursor_position(position);
            let _ = terminal.show_cursor();
        } else {
            let _ = terminal.hide_cursor();
        }
        Ok(())
    }
}

/// Static look of one machine panel: title, border color and the status
/// shown when the machine has no animated message.
struct PanelTheme {
    title: &'static str,
    border: Color,
    idle_status: &'static str,
}

impl PanelTheme {
    fn new(title: &'static str, border: Color, idle_status: &'static str) -> Self {
        Self {
            title,
            border,
            idle_status,
        }
    }
}

/// A machine panel: one row per machine, with the animated per-machine
/// status in the last column. `selected` highlights a row (selection in
/// the Detected panel, delete mode in the Added panel). A recent beacon
/// pulses the row: bright background fading back to the base color.
fn render_machines(
    frame: &mut Frame,
    area: Rect,
    theme: PanelTheme,
    devices: &[Device],
    selected: Option<usize>,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
) {
    let now = Instant::now();
    let pulse_duration = Duration::from_millis(PULSE_MS);
    let rows = devices.iter().enumerate().map(|(i, d)| {
        let flash = flashes
            .get(&d.key)
            .filter(|f| now.duration_since(f.start) < flash_duration(f.level));
        let pulse = pulses
            .get(&d.key)
            .map(|t| now.duration_since(*t))
            .filter(|e| *e < pulse_duration);
        let status = if let Some(f) = flash {
            f.msg.clone()
        } else if pulse.is_some() {
            "beacon received".to_string()
        } else {
            theme.idle_status.to_string()
        };
        let style = row_style(theme.border, selected == Some(i), flash, pulse, now);
        Row::new([
            Cell::from(d.name.clone()),
            Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(d.port.to_string()),
            Cell::from(status),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Length(5),
            Constraint::Min(8),
        ],
    )
    .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
    .block(
        Block::bordered()
            .title(format!("{} — {}", theme.title, devices.len()))
            .border_style(Style::new().fg(theme.border)),
    );
    frame.render_widget(table, area);
}

/// Style of a machine row: the panel's base color, overridden by the
/// selection, an urgent blink (warning, security), a beacon pulse, or the
/// row's animated status, in that order of precedence.
fn row_style(
    base: Color,
    selected: bool,
    flash: Option<&RowFlash>,
    pulse: Option<Duration>,
    now: Instant,
) -> Style {
    if selected {
        return Style::new().add_modifier(Modifier::REVERSED);
    }
    // Urgent statuses keep their blink so they can never be masked.
    if let Some(f) = flash.filter(|f| matches!(f.level, Level::Warning | Level::Security)) {
        return flash_style(f, blink(now.duration_since(f.start)));
    }
    // Beacon received: the row lights up, then fades back to its base
    // color through three phases, so the user sees the app receiving.
    if let Some(elapsed) = pulse {
        let third = Duration::from_millis(PULSE_MS / 3);
        if elapsed < third {
            return Style::new().fg(Color::Black).bg(Color::Cyan);
        }
        if elapsed < third * 2 {
            return Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        }
        return Style::new().fg(base).add_modifier(Modifier::BOLD);
    }
    match flash {
        Some(f) => flash_style(f, blink(now.duration_since(f.start))),
        None => Style::new().fg(base),
    }
}

/// Style of a row's animated status, by level (`on` is the visible phase
/// of the blinking levels).
fn flash_style(f: &RowFlash, on: bool) -> Style {
    match f.level {
        Level::Info => Style::new().fg(Color::Cyan),
        Level::Success => Style::new().fg(Color::Black).bg(Color::Green),
        Level::Warning => {
            if on {
                Style::new().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            }
        }
        Level::Security => {
            if on {
                Style::new().fg(Color::White).bg(Color::Red)
            } else {
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
            }
        }
    }
}

/// How long the animated status of a message of this level lasts.
fn flash_duration(level: Level) -> Duration {
    Duration::from_millis(match level {
        Level::Security => SECURITY_FLASH_MS,
        _ => FLASH_MS,
    })
}

/// Alternating on/off phase of a blinking row status.
fn blink(elapsed: Duration) -> bool {
    elapsed.as_millis() / BLINK_MS as u128 % 2 == 0
}

/// How a pressed key appears in the hint line's `[...]` tokens (`s`,
/// `Enter`, `Dn`, ...), or `None` when it appears in none of them.
fn key_label(code: KeyCode) -> Option<String> {
    match code {
        KeyCode::Char(c) => Some(c.to_string()),
        KeyCode::Enter => Some("Enter".to_string()),
        KeyCode::Esc => Some("Esc".to_string()),
        KeyCode::Up => Some("Up".to_string()),
        KeyCode::Down => Some("Dn".to_string()),
        KeyCode::PageUp => Some("PgUp".to_string()),
        KeyCode::PageDown => Some("PgDn".to_string()),
        _ => None,
    }
}

/// A hint line with its `[key]` tokens split into spans; the token holding
/// the last pressed key is highlighted for a short moment (bright first,
/// then bold only), so the user sees the app react to the key press.
fn hint_spans(text: &str, key_flash: Option<&KeyFlash>) -> Line<'static> {
    let Some(flash) = key_flash else {
        return Line::from(text.to_string());
    };
    let elapsed = Instant::now().duration_since(flash.start);
    if elapsed >= Duration::from_millis(KEY_FLASH_MS) {
        return Line::from(text.to_string());
    }
    let lit = || -> Style {
        // Lit only while recent, then a fading tail so the effect decays
        // instead of switching off in one frame.
        if elapsed < Duration::from_millis(KEY_FLASH_MS / 2) {
            Style::new()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        }
    };
    let mut spans: Vec<Span> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let end = start + end;
        if start > 0 {
            spans.push(Span::from(rest[..start].to_string()));
        }
        let token = &rest[start..=end];
        let inner = &rest[start + 1..end];
        let style = if inner.split('/').any(|k| k == flash.key) {
            lit()
        } else {
            Style::new()
        };
        spans.push(Span::styled(token.to_string(), style));
        rest = &rest[end + 1..];
    }
    spans.push(Span::from(rest.to_string()));
    Line::from(spans)
}

/// Activity pane: bottom-anchored log, shifted up by `scroll` lines. While
/// verbose mode is on, the pane's title says so.
fn render_log(frame: &mut Frame, area: Rect, log: &[String], scroll: u16, verbose: bool) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = log
        .len()
        .saturating_sub(visible + scroll as usize)
        .min(log.len());
    let lines: Vec<Line> = log
        .iter()
        .map(|msg| Line::styled(msg.clone(), log_style(msg)))
        .collect();
    let title = if verbose { "Activity — verbose" } else { "Activity" };
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title(title))
        .scroll((skip as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Help panel (key '?'): the CLI help text, replacing the machine panels
/// and the activity pane while shown. Section headers are bold; the pane
/// is bottom-anchored, shifted up by `scroll` lines.
fn render_help(frame: &mut Frame, area: Rect, scroll: u16) {
    let lines: Vec<Line> = usage_text()
        .lines()
        .map(|l| {
            if l.ends_with(':') {
                Line::styled(l.to_string(), Style::new().bold())
            } else {
                Line::from(l.to_string())
            }
        })
        .collect();
    let total = lines.len();
    let visible = area.height.saturating_sub(2) as usize;
    let skip = total.saturating_sub(visible + scroll as usize).min(total);
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title("Help"))
        .scroll((skip as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Color of a log line, derived from its content.
fn log_style(msg: &str) -> Style {
    if msg.contains("SECURITY") || msg.contains("ERROR") {
        Style::new().fg(Color::Red)
    } else if msg.starts_with("WARNING") || msg.starts_with("  !") {
        Style::new().fg(Color::Yellow)
    } else if msg.starts_with("[~]") {
        Style::new().fg(Color::Cyan)
    } else if msg.starts_with("[DRY RUN]") {
        Style::new().fg(Color::Magenta)
    } else if msg.starts_with("  +") {
        Style::new().fg(Color::Green)
    } else {
        Style::new()
    }
}

/// Question block, always the bottom block of the screen. Returns the text
/// cursor position when the user is typing a name. While the help panel is
/// shown (`help`), the block describes the help panel instead.
fn render_question(
    frame: &mut Frame,
    area: Rect,
    question: &Question,
    ctx: &QuestionCtx,
) -> Option<Position> {
    // The status hints mention the verbose key with its current state.
    let verbose_hint = format!("[v] verbose: {}", if ctx.verbose { "on" } else { "off" });
    let (title, border, question_line, hint_line) = if ctx.help
        && matches!(question, Question::Idle)
    {
        (
            "Help",
            Color::Cyan,
            "Command-line and interactive help".to_string(),
            hint_spans(
                "[Up/Dn/PgUp/PgDn] scroll help   [?] close   [s] sync git   [q] quit",
                ctx.key_flash,
            ),
        )
    } else {
        match question {
        Question::Idle => {
            if ctx.pending > 0 {
                (
                    "Status",
                    Color::Yellow,
                    format!("{} machine(s) detected — add them to your configuration", ctx.pending),
                    hint_spans(
                        &format!(
                            "[Up/Dn] select   [Enter/y] add   [n] ignore   [s] sync git   [d] delete   [w] wipe   [c] setup-cd   {verbose_hint}   [?] help   [q] quit"
                        ),
                        ctx.key_flash,
                    ),
                )
            } else {
                (
                    "Status",
                    Color::Blue,
                    "Waiting for devices…".to_string(),
                    hint_spans(
                        &format!(
                            "[s] sync git   [d] delete   [w] wipe   [c] setup-cd   {verbose_hint}   [?] help   [PgUp/PgDn] scroll activity   [q] quit"
                        ),
                        ctx.key_flash,
                    ),
                )
            }
        }
        Question::AddDevice { name, ip } => (
            "New device",
            Color::Yellow,
            format!("Add {name} ({ip}) to config?"),
            hint_spans(
                &format!("[y] add   [n] skip   [s] sync git   {verbose_hint}   [q] quit"),
                ctx.key_flash,
            ),
        ),
        Question::TrustKey { name } => (
            "Signing key changed",
            Color::Red,
            format!(
                "Trust the new signing key for {name}? (expected after a reinstall — otherwise it may be spoofing)"
            ),
            hint_spans("[y] trust   [n] reject", ctx.key_flash),
        ),
        Question::Rename { name, suggested } => (
            "Name already used",
            Color::Cyan,
            format!("New name for {name} ([Enter] = {suggested}, [Esc] = skip)"),
            Line::from(format!("> {}", ctx.input)),
        ),
        Question::Delete { name } => (
            "Delete device",
            Color::Red,
            format!("Delete {name} from configuration?"),
            hint_spans("[Up/Dn] select   [Enter] confirm   [Esc] cancel", ctx.key_flash),
        ),
        Question::Wipe { count } => (
            "Wipe configuration",
            Color::Red,
            format!(
                "Delete the ENTIRE configuration ({count} device block(s) + ~/.regroup-config)?"
            ),
            hint_spans("[Enter/y] confirm   [Esc/w/q] cancel", ctx.key_flash),
        ),
        Question::SetupCd { file } => (
            "Install regroup-cd",
            Color::Yellow,
            format!(
                "Install the 'regroup-cd' shell function (cd to ~/.regroup-config) in ~/{file}?"
            ),
            hint_spans("[Enter/y] confirm   [Esc/c/q] cancel", ctx.key_flash),
        ),
        Question::InstallKeySelectMachine { machines } => (
            "Install SSH key",
            Color::Green,
            format!("Select target machine ({} available)", machines.len()),
            hint_spans("[Up/Dn] select   [Enter] confirm   [Esc/q] cancel", ctx.key_flash),
        ),
        Question::InstallKeySelectUser { machine, users } => (
            "Install SSH key",
            Color::Green,
            format!("Select user on {machine} ({} available)", users.len()),
            hint_spans("[Up/Dn] select   [Enter] confirm   [Esc] back", ctx.key_flash),
        ),
        Question::InstallKeyConfirm { from_user, from_machine, to_user, to_machine } => (
            "Install SSH key",
            Color::Green,
            format!("Install key from {}@{} to {}@{}?", from_user, from_machine, to_user, to_machine),
            hint_spans("[y] confirm   [n/Esc] cancel", ctx.key_flash),
        ),
        Question::RemoveKeySelectConnection { connections } => (
            "Remove SSH key",
            Color::Red,
            format!("Select connection to remove ({} available)", connections.len()),
            hint_spans("[Up/Dn] select   [Enter] confirm   [Esc/q] cancel", ctx.key_flash),
        ),
        Question::RemoveKeyConfirm { connection_id, from_user, from_machine, to_user, to_machine } => (
            "Remove SSH key",
            Color::Red,
            format!("Remove key {} ({}@{}) from {}@{}?", connection_id, from_user, from_machine, to_user, to_machine),
            hint_spans("[y] confirm   [n/Esc] cancel", ctx.key_flash),
        ),
    }
    };
    let text = Text::from(vec![
        Line::styled(question_line, Style::new().bold()),
        hint_line,
    ]);
    frame.render_widget(
        Paragraph::new(text).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(border)),
        ),
        area,
    );
    if matches!(question, Question::Rename { .. }) {
        // Second line inside the block: "> " prefix plus the typed name.
        let x = area.x + 1 + 2 + ctx.input.chars().count() as u16;
        let y = area.y + 2;
        Some(Position::new(x, y))
    } else {
        None
    }
}

/// Plain line-based fallback for non-terminal stdout, mirroring the
/// interface the tool had before the full-screen ui.
pub struct PlainUi {
    rx: Receiver<String>,
    question: Question,
}

impl PlainUi {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut input = String::new();
            loop {
                input.clear();
                match io::stdin().read_line(&mut input) {
                    Ok(0) | Err(_) => break, // EOF: no more input
                    Ok(_) => {
                        if tx.send(input.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            rx,
            question: Question::Idle,
        }
    }

    fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            print!("{} ", self.prompt());
            let _ = io::stdout().flush();
        }
    }

    /// Prompt text for the current question.
    fn prompt(&self) -> String {
        match &self.question {
            Question::Idle => "[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, v=verbose, h=help]:".to_string(),
            Question::AddDevice { name, ip } => {
                format!("Add {name} ({ip}) to config? [y/N/q/s]:")
            }
            Question::TrustKey { name } => {
                format!("Trust the new signing key for {name}? [y/N]:")
            }
            Question::Rename { name, suggested } => {
                format!("New name for {name} (empty = {suggested}):")
            }
            Question::Delete { name } => format!("Delete {name}? [y/N]:"),
            Question::Wipe { count } => {
                format!("Wipe the ENTIRE configuration ({count} device block(s))? [y/N]:")
            }
            Question::SetupCd { file } => {
                format!("Install 'regroup-cd' in ~/{file}? [y/N]:")
            }
            Question::InstallKeySelectMachine { machines } => {
                format!("Select target machine for key installation ({} available):", machines.len())
            }
            Question::InstallKeySelectUser { machine, users } => {
                format!("Select user on {} for key installation ({} available):", machine, users.len())
            }
            Question::InstallKeyConfirm { from_user, from_machine, to_user, to_machine } => {
                format!("Install SSH key from {}@{} to {}@{}? [y/N]:", from_user, from_machine, to_user, to_machine)
            }
            Question::RemoveKeySelectConnection { connections } => {
                format!("Select connection to remove ({} available):", connections.len())
            }
            Question::RemoveKeyConfirm { connection_id, .. } => {
                format!("Remove SSH key {connection_id}? [y/N]:")
            }
        }
    }

    fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        let line = self.rx.recv_timeout(timeout).ok()?;
        let line = line.trim();
        match (&self.question, line) {
            (Question::Idle, "q" | "quit") => Some(Action::Quit),
            (Question::Idle, "s" | "sync") => Some(Action::Sync),
            (Question::Idle, "d") => Some(Action::DeleteMode),
            (Question::Idle, "w" | "wipe") => Some(Action::WipeMode),
            (Question::Idle, "c" | "cd") => Some(Action::SetupCdMode),
            (Question::Idle, "k" | "connections") => Some(Action::ConnectionsMode),
            (Question::Idle, "v" | "verbose") => Some(Action::ToggleVerbose),
            (Question::Idle, "h" | "help" | "?") => Some(Action::Help),
            (Question::Idle, _) => None,
            (Question::AddDevice { .. }, "y" | "yes") => Some(Action::AddCurrent),
            (Question::AddDevice { .. }, "s") => Some(Action::Sync),
            (Question::AddDevice { .. }, "d") => Some(Action::DeleteMode),
            (Question::AddDevice { .. }, "w") => Some(Action::WipeMode),
            (Question::AddDevice { .. }, "c") => Some(Action::SetupCdMode),
            (Question::AddDevice { .. }, "k") => Some(Action::ConnectionsMode),
            (Question::AddDevice { .. }, "q") => Some(Action::Quit),
            (Question::AddDevice { .. }, _) => Some(Action::SkipCurrent),
            (Question::TrustKey { .. }, "y" | "yes") => Some(Action::TrustKey),
            (Question::TrustKey { .. }, _) => Some(Action::RejectKey),
            (Question::Rename { suggested, .. }, "") => Some(Action::Rename(suggested.clone())),
            (Question::Rename { .. }, "q") => Some(Action::SkipRename),
            (Question::Rename { .. }, name) => Some(Action::Rename(name.to_string())),
            (Question::Delete { .. }, "y" | "yes") => Some(Action::DeleteConfirm),
            (Question::Delete { .. }, _) => Some(Action::DeleteCancel),
            (Question::Wipe { .. }, "y" | "yes") => Some(Action::WipeConfirm),
            (Question::Wipe { .. }, _) => Some(Action::WipeCancel),
            (Question::SetupCd { .. }, "y" | "yes") => Some(Action::SetupCdConfirm),
            (Question::SetupCd { .. }, _) => Some(Action::SetupCdCancel),
            // Key installation/removal actions
            (Question::InstallKeySelectMachine { .. }, "q") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectMachine { .. }, _) => Some(Action::InstallKeyMode),
            (Question::InstallKeySelectUser { .. }, "q") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectUser { .. }, _machine) => {
                // This would require more context to handle properly
                Some(Action::InstallKeyMode)
            }
            (Question::InstallKeyConfirm { .. }, "y" | "yes") => Some(Action::InstallKeyConfirm),
            (Question::InstallKeyConfirm { .. }, _) => Some(Action::InstallKeyCancel),
            (Question::RemoveKeySelectConnection { .. }, "q") => Some(Action::RemoveKeyCancel),
            (Question::RemoveKeySelectConnection { .. }, _) => Some(Action::RemoveKeyMode),
            (Question::RemoveKeyConfirm { .. }, "y" | "yes") => Some(Action::RemoveKeyConfirm),
            (Question::RemoveKeyConfirm { .. }, _) => Some(Action::RemoveKeyCancel),
        }
    }
}
