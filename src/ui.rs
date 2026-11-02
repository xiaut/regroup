//! Terminal user interface for interactive mode.
//!
//! Two backends behind the same interface:
//! - a full-screen ratatui interface when stdout is a terminal: devices
//!   panel, activity log, and the current question as the bottom block — the
//!   question can never be pushed away by notifications;
//! - a plain line-based fallback otherwise (piped output): prompts are
//!   printed as they change and answers are read as input lines.
//!
//! All modules display messages through [`log`]: before the full-screen ui
//! runs (startup, plain mode) messages go to stdout; afterwards they are
//! buffered in a channel and shown in the activity pane.

use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};

/// Maximum number of activity messages kept for display.
const LOG_CAP: usize = 1000;

/// Sender side of the log channel; unset until [`Ui::start`] creates it.
static LOG_TX: OnceLock<mpsc::Sender<String>> = OnceLock::new();

/// Display a message: in the activity pane once the full-screen ui runs,
/// on stdout otherwise.
pub fn log(msg: &str) {
    match LOG_TX.get() {
        Some(tx) => {
            let _ = tx.send(msg.to_string());
        }
        None => println!("{msg}"),
    }
}

/// Best-effort terminal restoration (raw mode off, leave alternate screen),
/// e.g. before `process::exit` from a code path that cannot run destructors.
pub fn restore() {
    ratatui::restore();
}

/// One machine for the devices panel.
#[derive(Clone)]
pub struct Device {
    pub name: String,
    pub ip: Option<String>,
    pub port: u16,
    /// Device discovered but not confirmed yet.
    pub pending: bool,
}

/// What the interactive loop is asking right now. The ui renders it as the
/// bottom block and derives the active keys from it.
#[derive(Clone, PartialEq)]
pub enum Question {
    /// Nothing pending: only global keys.
    Idle,
    AddDevice { name: String, ip: String },
    TrustKey { name: String },
    Rename { name: String, suggested: String },
}

/// A user decision, translated by the ui from key presses (or, in plain
/// mode, from input lines).
pub enum Action {
    Quit,
    Refresh,
    AddCurrent,
    SkipCurrent,
    TrustKey,
    RejectKey,
    Rename(String),
    SkipRename,
}

/// The interface used by the interactive loop.
pub enum Ui {
    Tty(TtyUi),
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
            Ui::Tty(TtyUi {
                terminal,
                log_rx,
                log: Vec::new(),
                scroll: 0,
                question: Question::Idle,
                input: String::new(),
                devices: Vec::new(),
            })
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

    /// Update the devices panel (ignored in plain mode).
    pub fn set_devices(&mut self, devices: Vec<Device>) {
        if let Ui::Tty(ui) = self {
            ui.devices = devices;
        }
    }

    /// Wait up to `timeout` for a user decision.
    pub fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        match self {
            Ui::Tty(ui) => ui.poll_action(timeout),
            Ui::Plain(ui) => ui.poll_action(timeout),
        }
    }

    /// Refresh the display (no-op in plain mode).
    pub fn draw(&mut self) {
        if let Ui::Tty(ui) = self {
            let _ = ui.draw();
        }
    }
}

/// Full-screen ratatui interface.
pub struct TtyUi {
    terminal: DefaultTerminal,
    log_rx: Receiver<String>,
    log: Vec<String>,
    /// Lines scrolled up from the bottom of the activity pane; 0 follows it.
    scroll: u16,
    question: Question,
    /// Text typed for the rename question.
    input: String,
    devices: Vec<Device>,
}

impl Drop for TtyUi {
    fn drop(&mut self) {
        ratatui::restore();
        // Messages logged since the last draw are printed to stdout so
        // nothing is lost when the ui closes.
        for msg in self.log_rx.try_iter() {
            println!("{msg}");
        }
    }
}

impl TtyUi {
    fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            self.input.clear();
            self.scroll = 0;
        }
    }

    /// Move messages logged since the last draw into the pane, capped. Each
    /// line of a multi-line message becomes its own entry.
    fn drain_log(&mut self) {
        while let Ok(msg) = self.log_rx.try_recv() {
            for line in msg.split('\n') {
                self.log.push(line.to_string());
            }
        }
        let excess = self.log.len().saturating_sub(LOG_CAP);
        if excess > 0 {
            self.log.drain(..excess);
        }
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
    /// and do not produce actions.
    fn key_action(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            return Some(Action::Quit);
        }
        match key.code {
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
                (Question::Idle, 'r') => Some(Action::Refresh),
                (Question::AddDevice { .. }, 'y') => Some(Action::AddCurrent),
                (Question::AddDevice { .. }, 'n') => Some(Action::SkipCurrent),
                (Question::AddDevice { .. }, 'r') => Some(Action::Refresh),
                (Question::AddDevice { .. }, 'q') => Some(Action::Quit),
                (Question::TrustKey { .. }, 'y') => Some(Action::TrustKey),
                (Question::TrustKey { .. }, 'n') => Some(Action::RejectKey),
                (Question::Rename { .. }, c) => {
                    self.input.push(c);
                    None
                }
                _ => None,
            },
            KeyCode::Esc => match self.question {
                Question::TrustKey { .. } => Some(Action::RejectKey),
                Question::Rename { .. } => Some(Action::SkipRename),
                _ => None,
            },
            KeyCode::Backspace if matches!(self.question, Question::Rename { .. }) => {
                self.input.pop();
                None
            }
            KeyCode::Enter if matches!(self.question, Question::Rename { .. }) => {
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
            devices,
            ..
        } = self;
        let mut cursor = None;
        terminal.draw(|frame| {
            let [devices_area, log_area, question_area] = Layout::vertical([
                Constraint::Percentage(35),
                Constraint::Min(1),
                Constraint::Length(4),
            ])
            .areas(frame.area());
            render_devices(frame, devices_area, devices);
            render_log(frame, log_area, log, *scroll);
            cursor = render_question(frame, question_area, question, input);
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

/// Devices panel: one row per known or pending machine.
fn render_devices(frame: &mut Frame, area: Rect, devices: &[Device]) {
    let known = devices.iter().filter(|d| !d.pending).count();
    let rows = devices.iter().map(|d| {
        let style = if d.pending {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Green)
        };
        Row::new([
            Cell::from(d.name.clone()),
            Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(d.port.to_string()),
            Cell::from(if d.pending { "pending" } else { "known" }).style(style),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(45),
            Constraint::Percentage(35),
            Constraint::Length(6),
            Constraint::Length(9),
        ],
    )
    .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
    .block(Block::bordered().title(format!("Devices — {known} known")));
    frame.render_widget(table, area);
}

/// Activity pane: bottom-anchored log, shifted up by `scroll` lines.
fn render_log(frame: &mut Frame, area: Rect, log: &[String], scroll: u16) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = log
        .len()
        .saturating_sub(visible + scroll as usize)
        .min(log.len());
    let lines: Vec<Line> = log
        .iter()
        .map(|msg| Line::styled(msg.clone(), log_style(msg)))
        .collect();
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title("Activity"))
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
/// cursor position when the user is typing a name.
fn render_question(
    frame: &mut Frame,
    area: Rect,
    question: &Question,
    input: &str,
) -> Option<Position> {
    let (title, border, question_line, hint_line) = match question {
        Question::Idle => (
            "Status",
            Color::Blue,
            "Waiting for devices…".to_string(),
            Line::from("[r] refresh git   [q] quit   [Up/Dn] scroll activity"),
        ),
        Question::AddDevice { name, ip } => (
            "New device",
            Color::Yellow,
            format!("Add {name} ({ip}) to config?"),
            Line::from("[y] add   [n] skip   [r] refresh git   [q] quit"),
        ),
        Question::TrustKey { name } => (
            "Signing key changed",
            Color::Red,
            format!("Trust the new signing key for {name}?"),
            Line::from("[y] trust   [n] reject"),
        ),
        Question::Rename { name, suggested } => (
            "Name already used",
            Color::Cyan,
            format!("New name for {name} ([Enter] = {suggested}, [Esc] = skip)"),
            Line::from(format!("> {input}")),
        ),
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
        let x = area.x + 1 + 2 + input.chars().count() as u16;
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
            Question::Idle => "[q=quit, r=refresh git]:".to_string(),
            Question::AddDevice { name, ip } => {
                format!("Add {name} ({ip}) to config? [y/N/q/r]:")
            }
            Question::TrustKey { name } => {
                format!("Trust the new signing key for {name}? [y/N]:")
            }
            Question::Rename { name, suggested } => {
                format!("New name for {name} (empty = {suggested}):")
            }
        }
    }

    fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        let line = self.rx.recv_timeout(timeout).ok()?;
        let line = line.trim();
        match (&self.question, line) {
            (Question::Idle, "q" | "quit") => Some(Action::Quit),
            (Question::Idle, "r" | "refresh") => Some(Action::Refresh),
            (Question::Idle, _) => None,
            (Question::AddDevice { .. }, "y" | "yes") => Some(Action::AddCurrent),
            (Question::AddDevice { .. }, "r") => Some(Action::Refresh),
            (Question::AddDevice { .. }, "q") => Some(Action::Quit),
            (Question::AddDevice { .. }, _) => Some(Action::SkipCurrent),
            (Question::TrustKey { .. }, "y" | "yes") => Some(Action::TrustKey),
            (Question::TrustKey { .. }, _) => Some(Action::RejectKey),
            (Question::Rename { suggested, .. }, "") => Some(Action::Rename(suggested.clone())),
            (Question::Rename { .. }, "q") => Some(Action::SkipRename),
            (Question::Rename { .. }, name) => Some(Action::Rename(name.to_string())),
        }
    }
}
