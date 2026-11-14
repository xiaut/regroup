//! Full-screen terminal UI implementation using ratatui.
//!
//! This module provides the TtyUi struct which handles the full-screen TUI
//! interface, including device panels, activity log, and question display.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::DefaultTerminal;

use super::keys::{resolve, Context, KeyResolution};
use super::render::{
    count_wrapped_lines, help_hint_text, idle_hint_text, key_label, modal_hint_text,
    render_all_panes, render_help, render_question, KeyFlash, QuestionCtx, RowFlash,
};
use super::types::{
    bindings_hint_text, ActivePane, Action, NavFocus, Navigation, Question, UiDevice, UiEvent,
    UiUser, UserStatus,
};

/// Calculate the height needed for the question block based on its content.
/// Returns the height in lines, including borders.
fn calculate_question_height(question: &Question, ctx: &QuestionCtx, width: u16) -> u16 {
    // The question block has: title border (1) + content + bottom border (1)
    // Content has: question_line (1 or more when wrapped) + second line (1 or
    // more when wrapped): the hint line, or the typed input for text questions
    // Minimum height is 4: top border + question + second line + bottom border
    
    if width <= 2 {
        return 4; // Minimum height
    }
    
    let content_width = width.saturating_sub(2) as usize; // Subtract border characters
    
    // Get both the question line text and hint text for the current question
    // We need to use owned Strings because format! returns String
    let (question_line_text, hint_text): (String, String) = match question {
        Question::Idle(_) if ctx.help => {
            (
                "Command-line and interactive help".to_string(),
                help_hint_text(),
            )
        }
        Question::Idle(_) => {
            // Hint line driven by the navigation key table, same as render_question
            let status_line = ctx.active_pane.name().to_string();
            (status_line, idle_hint_text(Context::Pane {
                focus: ctx.navigation.focus,
                depth: ctx.navigation.depth,
                confirmed_user: ctx.confirmed_user,
            }))
        }
        Question::AddDevice(def, name, ip) => {
            (
                format!("Add {name} ({ip}) to config?"),
                modal_hint_text(def),
            )
        }
        Question::TrustKey(def, name) => {
            (
                format!("Trust the new signing key for {name}? (expected after a reinstall — otherwise it may be spoofing)"),
                modal_hint_text(def),
            )
        }
        Question::Rename(def, name, suggested) => {
            // Text-input question: the wrapped question line, then the
            // typed input on its own (possibly wrapped) line.
            let question_line = format!(
                "New name for {name} ({}; empty = {suggested})",
                bindings_hint_text(def.bindings)
            );
            let input_line = format!("> {}", ctx.input);
            let lines = count_wrapped_lines(&question_line, content_width, false)
                + count_wrapped_lines(&input_line, content_width, false);
            return (2 + lines as u16).clamp(3, 10);
        }
        Question::AddTempUser(def, device) => {
            // Input line handled like the rename question
            let question_line = format!(
                "Temporary user on {device} (in memory only) — {}",
                bindings_hint_text(def.bindings)
            );
            let input_line = format!("> {}", ctx.input);
            let lines = count_wrapped_lines(&question_line, content_width, false)
                + count_wrapped_lines(&input_line, content_width, false);
            return (2 + lines as u16).clamp(3, 10);
        }
        Question::Delete(def, name) => {
            (
                format!("Delete {name} from configuration?"),
                modal_hint_text(def),
            )
        }
        Question::Wipe(def, count) => {
            (
                format!("Delete the ENTIRE configuration ({count} device block(s) + ~/.regroup-config)?"),
                modal_hint_text(def),
            )
        }
        Question::SetupCd(def, file) => {
            (
                format!("Install the 'regroup-cd' shell function (cd to ~/.regroup-config) in ~/{file}?"),
                modal_hint_text(def),
            )
        }
    };
    
    // Calculate how many lines each part will take when wrapped
    // Question line uses word-based wrapping, hint line uses token-based wrapping
    let question_lines = count_wrapped_lines(&question_line_text, content_width, false);
    let hint_lines = count_wrapped_lines(&hint_text, content_width, true);
    
    // Total content lines = question lines + hint lines
    let total_content_lines = question_lines + hint_lines;
    
    // Total height: top border (1) + content lines + bottom border (1)
    let total_height = 2 + total_content_lines as u16;
    
    total_height.clamp(3, 10) // Clamp between 3 and 10 lines
}

use crate::cli::usage_text;

/// Full-screen ratatui interface.
pub struct TtyUi {
    pub terminal: DefaultTerminal,
    pub log_rx: Receiver<UiEvent>,
    pub log: Vec<String>,
    /// Lines scrolled up from the bottom of the logs pane; 0 follows it.
    pub log_scroll: u16,
    pub question: Question,
    /// Text typed for the rename question.
    pub input: String,
    /// Devices announced but not accepted yet (Detected panel).
    pub detected: Vec<UiDevice>,
    /// Devices in the configuration (Added panel).
    pub added: Vec<UiDevice>,
    /// Row of the Detected panel highlighted by the selection (passed from state).
    pub detected_selected: Option<usize>,
    /// Row of the Added panel highlighted by the selection (passed from state).
    pub added_selected: Option<usize>,
    /// Row of the Added panel highlighted in delete mode (passed from state).
    pub delete_selected: Option<usize>,
    /// Users of the device selected in the Added panel (Users panel).
    pub users: Vec<UiUser>,
    /// Row of the Users panel highlighted by the selection (passed from state).
    pub users_selected: Option<usize>,
    /// Navigation state: focus + depth in the context hierarchy.
    pub navigation: Navigation,
    /// Which pane is currently active for navigation and actions (derived from navigation).
    pub active_pane: ActivePane,
    /// Whether the help panel is shown (key '?'): it replaces the device
    /// panels and the logs pane; the question block stays interactive.
    pub show_help: bool,
    /// Lines scrolled up from the bottom of the help panel; 0 follows it.
    pub help_scroll: u16,
    /// Animated per-device statuses, by SSH public key.
    pub flashes: HashMap<String, RowFlash>,
    /// Rows currently pulsing after a received beacon, by SSH public key.
    pub pulses: HashMap<String, Instant>,
    /// Last key press, briefly highlighted in the hint line.
    pub key_flash: Option<KeyFlash>,
}

impl Drop for TtyUi {
    fn drop(&mut self) {
        ratatui::restore();
        // Messages logged since the last draw are printed to stdout so
        // nothing is lost when the ui closes.
        for event in self.log_rx.try_iter() {
            match event {
                UiEvent::Global(msg) => println!("{msg}"),
                UiEvent::UiDevice {
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
    /// Set the current question. The full-screen ui clears the typed input
    /// and resets the logs scroll whenever the question changes.
    pub fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            self.input.clear();
            self.log_scroll = 0;
            // An urgent question takes precedence over the help panel:
            // close it so the question gets the full screen back.
            if !matches!(self.question, Question::Idle(_)) {
                self.show_help = false;
                self.help_scroll = 0;
            }
        }
    }

    /// Move messages logged since the last draw into the pane, capped.
    /// General messages become activity lines; device-specific messages
    /// become row flashes (security messages are additionally mirrored into
    /// the activity pane so they do not vanish with the animation).
    pub fn drain_log(&mut self) {
        while let Ok(event) = self.log_rx.try_recv() {
            match event {
                UiEvent::Global(msg) => {
                    for line in msg.split('\n') {
                        self.log.push(line.to_string());
                    }
                }
                UiEvent::UiDevice {
                    key,
                    level,
                    msg,
                    detail,
                    ..
                } => {
                    if level == super::types::Level::Security {
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
        let excess = self.log.len().saturating_sub(super::logging::LOG_CAP);
        if excess > 0 {
            self.log.drain(..excess);
        }
        // Keep only recent pulses of devices that are still displayed.
        let now = Instant::now();
        self.pulses.retain(|key, t| {
            (self.detected.iter().any(|d| d.key == *key)
                || self.added.iter().any(|d| d.key == *key))
                && now.duration_since(*t) < Duration::from_millis(super::logging::PULSE_MS)
        });
        // Keep only recent flashes of devices that are still displayed.
        self.flashes.retain(|key, f| {
            (self.detected.iter().any(|d| d.key == *key)
                || self.added.iter().any(|d| d.key == *key))
                && now.duration_since(f.start) < Duration::from_secs(60)
        });
    }

    /// Wait up to `timeout` for a user action.
    pub fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        self.drain_log();
        if !event::poll(timeout).unwrap_or(false) {
            return None;
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => self.key_action(key),
            _ => None,
        }
    }

    /// Translate one key press into an action.
    ///
    /// The key is resolved against the state's binding table
    /// ([`super::keys::bindings_for`]) — the same table the hint lines
    /// are built from — so a key that is not advertised is not handled,
    /// and every advertised key is. Keys marked internal (navigation,
    /// scrolling, help, text capture) are applied locally here.
    ///
    /// Every press is recorded so the hint line can highlight the key.
    pub fn key_action(&mut self, key: KeyEvent) -> Option<Action> {
        // Record key press for hint highlighting
        if let Some(label) = key_label(key.code) {
            self.key_flash = Some(KeyFlash {
                key: label,
                start: Instant::now(),
            });
        }

        // Ctrl+C or Ctrl+D always quits
        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            return Some(Action::Quit);
        }

        let confirmed_user = self
            .users_selected
            .and_then(|i| self.users.get(i))
            .is_some_and(|u| u.status != UserStatus::Temporary);
        let ctx = if self.show_help {
            Context::Help
        } else {
            let def = self.question.def();
            if matches!(self.question, Question::Rename(..) | Question::AddTempUser(..)) {
                Context::TextInput(def)
            } else if matches!(self.question, Question::Idle(_)) {
                Context::Pane {
                    focus: self.navigation.focus,
                    depth: self.navigation.depth,
                    confirmed_user,
                }
            } else {
                Context::Modal(def)
            }
        };
        match resolve(ctx, key.code) {
            // Not in the table: not handled at all.
            None => None,
            Some(KeyResolution::Action(action)) => Some(action),
            Some(KeyResolution::SubmitInput) => self.submit_input(),
            Some(KeyResolution::Internal) => self.apply_internal(key.code),
        }
    }

    /// Submit the typed text of a text-input question (Enter).
    fn submit_input(&mut self) -> Option<Action> {
        let input = std::mem::take(&mut self.input);
        match &self.question {
            // An empty name accepts the suggestion.
            Question::Rename(_, _, suggested) => Some(Action::Rename(if input.is_empty() {
                suggested.clone()
            } else {
                input
            })),
            // An empty user name cancels.
            Question::AddTempUser(_, _) => {
                if input.is_empty() {
                    Some(Action::CancelTempUser)
                } else {
                    Some(Action::TempUserName(input))
                }
            }
            _ => None,
        }
    }

    /// Apply a key the binding table marked as internal: navigation,
    /// list selection, logs scrolling, help panel, text capture.
    fn apply_internal(&mut self, code: KeyCode) -> Option<Action> {
        // Help panel: scroll it or close it.
        if self.show_help {
            match code {
                KeyCode::Up => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::Down => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_add(10),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.show_help = false;
                    self.help_scroll = 0;
                }
                _ => {}
            }
            return None;
        }

        // Text-input questions: capture the typed characters.
        if matches!(self.question, Question::Rename(..) | Question::AddTempUser(..)) {
            match code {
                KeyCode::Char(c) => self.input.push(c),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                _ => {}
            }
            return None;
        }

        // Navigation state machine (arrows, Enter, Esc).
        let has_users = !self.users.is_empty();
        let new_nav = self.navigation.handle_key(code, has_users);
        if new_nav != self.navigation {
            let old_focus = self.navigation.focus;
            self.navigation = new_nav;
            self.active_pane = new_nav.active_pane();

            // Clear selections when switching focus
            if new_nav.focus != old_focus {
                match new_nav.focus {
                    NavFocus::Detected => {
                        self.added_selected = None;
                        self.users_selected = None;
                    }
                    NavFocus::Added => {
                        self.detected_selected = None;
                        self.users_selected = None;
                    }
                    NavFocus::Users => {
                        self.detected_selected = None;
                    }
                    NavFocus::Logs => {
                        self.detected_selected = None;
                        self.added_selected = None;
                    }
                    NavFocus::Center => {
                        self.detected_selected = None;
                        self.added_selected = None;
                        self.users_selected = None;
                    }
                }
            }
            return None;
        }

        // List selection (Up/Down in fullscreen lists).
        if let Some(action) = self.navigation.list_nav_action(code) {
            return Some(action);
        }

        // Logs pane scrolling.
        if matches!(self.active_pane, ActivePane::Logs(_)) {
            match code {
                KeyCode::Up => {
                    self.log_scroll = self.log_scroll.saturating_add(1);
                    return None;
                }
                KeyCode::Down => {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                    return None;
                }
                KeyCode::PageUp => {
                    self.log_scroll = self.log_scroll.saturating_add(10);
                    return None;
                }
                KeyCode::PageDown => {
                    self.log_scroll = self.log_scroll.saturating_sub(10);
                    return None;
                }
                _ => {}
            }
        }

        // Help toggle.
        if code == KeyCode::Char('?') {
            self.show_help = !self.show_help;
            self.help_scroll = 0;
        }
        None
    }

    /// Draw the UI frame.
    pub fn draw(&mut self) -> io::Result<()> {
        self.drain_log();
        // Split borrows: the closure renders from the data while `draw`
        // mutably borrows the terminal.
        let Self {
            terminal,
            log,
            log_scroll,
            question,
            input,
            detected,
            added,
            detected_selected,
            added_selected,
            users,
            users_selected,
            show_help,
            help_scroll,
            active_pane,
            navigation,
            flashes,
            pulses,
            key_flash,
            ..
        } = self;
        let mut cursor = None;
        terminal.draw(|frame| {
            // Calculate the height needed for the question block
            let confirmed_user = users_selected
                .and_then(|i| users.get(i))
                .is_some_and(|u| u.status != UserStatus::Temporary);
            let question_ctx = QuestionCtx {
                input,
                pending: detected.len(),
                help: *show_help,
                active_pane: *active_pane,
                navigation: *navigation,
                confirmed_user,
                key_flash: key_flash.as_ref(),
            };
            let question_height = calculate_question_height(question, &question_ctx, frame.area().width);
            
            let [main_area, question_area] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(question_height),
            ])
            .areas(frame.area());
            if *show_help {
                // The help panel replaces the main area; the question block stays interactive.
                let visible = main_area.height.saturating_sub(2) as usize;
                let max_scroll = usage_text()
                    .lines()
                    .count()
                    .saturating_sub(visible) as u16;
                let help_scroll_bounded = (*help_scroll).min(max_scroll);
                render_help(frame, main_area, help_scroll_bounded);
            } else {
                // Always show all three panes (Detected, Added, Logs)
                // The active pane has a bold border
                render_all_panes(
                    frame,
                    main_area,
                    *active_pane,
                    navigation.focus,
                    detected,
                    *detected_selected,
                    added,
                    *added_selected,
                    log,
                    *log_scroll,
                    flashes,
                    pulses,
                    users,
                    *users_selected,
                );
            }
            cursor = render_question(frame, question_area, question, &question_ctx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{NavFocus, Navigation};

    fn ctx(input: &str) -> QuestionCtx<'_> {
        QuestionCtx {
            input,
            pending: 0,
            help: false,
            active_pane: ActivePane::main_menu(),
            navigation: Navigation {
                focus: NavFocus::Center,
                depth: None,
            },
            confirmed_user: false,
            key_flash: None,
        }
    }

    #[test]
    fn text_question_height_grows_when_the_question_wraps() {
        let long_name = "a-very-long-device-hostname-that-wraps".to_string();
        let q = Question::rename(long_name, "suggested".to_string());
        // Wide terminal: question and input each fit on one line.
        assert_eq!(calculate_question_height(&q, &ctx(""), 120), 4);
        // Narrow terminal: the question line wraps, the block must grow so
        // the input line stays visible.
        let narrow = calculate_question_height(&q, &ctx(""), 30);
        assert!(narrow > 4, "height {narrow} should grow past the fixed 4");
        assert!(narrow <= 10, "height is clamped at 10");
    }

    #[test]
    fn text_question_height_counts_a_wrapped_input_line() {
        let q = Question::add_temp_user("dev".to_string());
        // At width 80 (content 78) the question fits on one line but a
        // 100-char input wraps, so the block is taller than the minimum.
        let input = "x".repeat(100);
        let h = calculate_question_height(&q, &ctx(&input), 80);
        assert_eq!(h, 2 + 1 + 2);
    }

    #[test]
    fn action_question_height_is_unchanged() {
        let q = Question::wipe(3);
        assert_eq!(calculate_question_height(&q, &ctx(""), 120), 4);
    }
}
