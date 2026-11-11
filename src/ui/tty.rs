//! Full-screen terminal UI implementation using ratatui.
//!
//! This module provides the TtyUi struct which handles the full-screen TUI
//! interface, including device panels, activity log, and question display.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::DefaultTerminal;

use super::render::{key_label, render_all_panes, render_help, render_question, KeyFlash, QuestionCtx, RowFlash};
use super::types::{ActivePane, Action, Device, Question, UiEvent};

/// Calculate the height needed for the question block based on its content.
/// Returns the height in lines, including borders.
fn calculate_question_height(question: &Question, ctx: &QuestionCtx, width: u16) -> u16 {
    // The question block has: title border (1) + content + bottom border (1)
    // Content has: question_line (1) + hint_line (1 or more when wrapped)
    // Minimum height is 4: top border + question + hint + bottom border
    
    // Get the hint text for the current question
    let hint_text = match question {
        Question::Idle if ctx.help => {
            "[Up/Dn/PgUp/PgDn] scroll help   [?] close   [s] sync git   [q] quit"
        }
        Question::Idle => {
            match ctx.active_pane {
                ActivePane::MainMenu => "[d] Detected   [a] Added   [l] Logs   [?] help   [q] quit",
                ActivePane::Detected if ctx.pending > 0 => 
                    "[Esc] no pane   [Up/Dn] select   [a] add   [n] ignore   [?] help   [q] quit",
                ActivePane::Detected => 
                    "[Esc] no pane   [?] help   [q] quit",
                ActivePane::Added => 
                    "[Esc] no pane   [Up/Dn] select   [i] install key   [r] remove key   [d] delete   [s] sync   [w] wipe   [c] setup-cd   [?] help   [q] quit",
                ActivePane::Logs => 
                    "[Esc] no pane   [Up/Dn] scroll line   [PgUp/PgDn] scroll page   [v] toggle verbose   [?] help   [q] quit",
            }
        }
        Question::AddDevice { .. } => {
            return 4; // These are shorter hints, 4 lines should be enough
        }
        Question::TrustKey { .. } => "[y] trust   [n] reject",
        Question::Rename { .. } => return 4, // Special case with input line
        Question::Delete { .. } => "[Up/Dn] select   [Enter] confirm   [Esc] cancel",
        Question::Wipe { .. } => "[Enter/y] confirm   [Esc/w/q] cancel",
        Question::SetupCd { .. } => "[Enter/y] confirm   [Esc/c/q] cancel",
        Question::InstallKeySelectMachine { .. } => "[Up/Dn] select   [Enter] confirm   [Esc/q] cancel",
        Question::InstallKeySelectUser { .. } => "[Up/Dn] select   [Enter] confirm   [Esc] back",
        Question::InstallKeyConfirm { .. } => "[y] confirm   [n/Esc] cancel",
        Question::RemoveKeySelectConnection { .. } => "[Up/Dn] select   [Enter] confirm   [Esc/q] cancel",
        Question::RemoveKeyConfirm { .. } => "[y] confirm   [n/Esc] cancel",
    };
    
    // Calculate how many lines the hint will take when wrapped to fit in (width - 2 for borders)
    // Hints are formatted as "[key] description   [key] description" where each
    // [key] description pair must stay together (non-breakable span).
    // They are separated by "   " (three spaces).
    if width <= 2 {
        return 4; // Minimum height
    }
    let content_width = width.saturating_sub(2); // Subtract border characters
    
    // Split hint text into tokens separated by "   "
    let tokens: Vec<&str> = hint_text.split("   ").collect();
    
    // Count how many lines the hint will take, treating each token as non-breakable
    let mut lines = 1;
    let mut current_line_width = 0;
    
    for token in tokens {
        let token_len = token.len();
        
        if current_line_width == 0 {
            // First token on this line
            current_line_width = token_len;
        } else if current_line_width + 3 + token_len <= content_width as usize {
            // Token fits on current line (account for the "   " separator)
            current_line_width += 3 + token_len;
        } else {
            // Token doesn't fit, move to next line
            lines += 1;
            current_line_width = token_len;
        }
    }
    
    // Total height: top border (1) + question line (1) + hint lines + bottom border (1)
    // We always have at least 1 line for question and 1 for hint
    let total_content_lines = 1 + lines; // question line + hint lines
    let total_height = 2 + total_content_lines; // + 2 for borders
    
    total_height.min(10).max(3) // Clamp between 3 and 10 lines
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
    /// Machines announced but not accepted yet (Detected panel).
    pub detected: Vec<Device>,
    /// Machines in the configuration (Added panel).
    pub added: Vec<Device>,
    /// Row of the Detected panel highlighted by the selection (passed from state).
    pub detected_selected: Option<usize>,
    /// Row of the Added panel highlighted by the selection (passed from state).
    pub added_selected: Option<usize>,
    /// Row of the Added panel highlighted in delete mode (passed from state).
    pub delete_selected: Option<usize>,
    /// Which pane is currently active for navigation and actions.
    pub active_pane: ActivePane,
    /// Whether the help panel is shown (key '?'): it replaces the machine
    /// panels and the logs pane; the question block stays interactive.
    pub show_help: bool,
    /// Lines scrolled up from the bottom of the help panel; 0 follows it.
    pub help_scroll: u16,
    /// Verbose mode: reported in the logs pane title and the status
    /// hints. The interactive loop keeps it in sync with its own state.
    pub verbose: bool,
    /// Animated per-machine statuses, by SSH public key.
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
    /// Set the current question. The full-screen ui clears the typed input
    /// and resets the logs scroll whenever the question changes.
    pub fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            self.input.clear();
            self.log_scroll = 0;
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
    pub fn drain_log(&mut self) {
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
        // Keep only recent pulses of machines that are still displayed.
        let now = Instant::now();
        self.pulses.retain(|key, t| {
            (self.detected.iter().any(|d| d.key == *key)
                || self.added.iter().any(|d| d.key == *key))
                && now.duration_since(*t) < Duration::from_millis(super::logging::PULSE_MS)
        });
        // Keep only recent flashes of machines that are still displayed.
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

    /// Translate one key press into an action. Scroll keys are handled here
    /// and do not produce actions. Up/Down select a machine in the active
    /// panel. Tab switches between panes. Every press is recorded so the hint
    /// line can highlight the key.
    pub fn key_action(&mut self, key: KeyEvent) -> Option<Action> {
        if let Some(label) = key_label(key.code) {
            self.key_flash = Some(KeyFlash {
                key: label,
                start: Instant::now(),
            });
        }
        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && matches!(key.code, crossterm::event::KeyCode::Char('c') | crossterm::event::KeyCode::Char('d'))
        {
            return Some(Action::Quit);
        }
        let in_delete = matches!(self.question, Question::Delete { .. });
        let in_wipe = matches!(self.question, Question::Wipe { .. });
        let in_setup = matches!(self.question, Question::SetupCd { .. });
        let in_rename = matches!(self.question, Question::Rename { .. });
        
        // Check if we're in a modal question (not Idle)
        let in_modal = in_delete || in_wipe || in_setup || in_rename || self.show_help;
        
        // Escape returns to Main menu from any pane (unless in modal)
        if !in_modal && matches!(key.code, crossterm::event::KeyCode::Esc) {
            self.active_pane = ActivePane::MainMenu;
            self.detected_selected = None;
            self.added_selected = None;
            return Some(Action::BackToMainMenu);
        }
        
        // Handle help panel scrolling
        if self.show_help {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    self.help_scroll = self.help_scroll.saturating_add(1);
                    return None;
                }
                crossterm::event::KeyCode::Down => {
                    self.help_scroll = self.help_scroll.saturating_sub(1);
                    return None;
                }
                crossterm::event::KeyCode::PageUp => {
                    self.help_scroll = self.help_scroll.saturating_add(10);
                    return None;
                }
                crossterm::event::KeyCode::PageDown => {
                    self.help_scroll = self.help_scroll.saturating_sub(10);
                    return None;
                }
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('?') => {
                    self.show_help = false;
                    self.help_scroll = 0;
                    return None;
                }
                _ => {}
            }
        }
        
        // Handle modal questions (wipe, setup-cd, delete, trust key, rename)
        if in_wipe {
            match key.code {
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('y') => {
                    return Some(Action::WipeConfirm);
                }
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('n') => {
                    return Some(Action::WipeCancel);
                }
                crossterm::event::KeyCode::Char(c) => {
                    return (c == 'w' || c == 'q').then_some(Action::WipeCancel);
                }
                _ => return None,
            }
        }
        
        if in_setup {
            match key.code {
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('y') => {
                    return Some(Action::SetupCdConfirm);
                }
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('n') => {
                    return Some(Action::SetupCdCancel);
                }
                crossterm::event::KeyCode::Char(c) => {
                    return (c == 'c' || c == 'q').then_some(Action::SetupCdCancel);
                }
                _ => return None,
            }
        }
        
        if in_delete {
            match key.code {
                crossterm::event::KeyCode::Up => return Some(Action::SelectPrev),
                crossterm::event::KeyCode::Down => return Some(Action::SelectNext),
                crossterm::event::KeyCode::Enter => return Some(Action::DeleteConfirm),
                crossterm::event::KeyCode::Esc => return Some(Action::DeleteCancel),
                crossterm::event::KeyCode::Char(c) => {
                    return (c == 'd' || c == 'q').then_some(Action::DeleteCancel);
                }
                _ => return None,
            }
        }
        
        if in_rename {
            match key.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.input.push(c);
                    return None;
                }
                crossterm::event::KeyCode::Backspace => {
                    self.input.pop();
                    return None;
                }
                crossterm::event::KeyCode::Enter => {
                    let name = std::mem::take(&mut self.input);
                    return Some(Action::Rename(name));
                }
                crossterm::event::KeyCode::Esc => return Some(Action::SkipRename),
                _ => return None,
            }
        }
        
        // Handle TrustKey question
        if matches!(self.question, Question::TrustKey { .. }) {
            match key.code {
                crossterm::event::KeyCode::Char('y') => return Some(Action::TrustKey),
                crossterm::event::KeyCode::Char('n') | crossterm::event::KeyCode::Esc => {
                    return Some(Action::RejectKey);
                }
                _ => return None,
            }
        }
        
        // Handle AddDevice question (plain mode)
        if matches!(self.question, Question::AddDevice { .. }) {
            match key.code {
                crossterm::event::KeyCode::Char('y') => return Some(Action::AddCurrent),
                crossterm::event::KeyCode::Char('n') => return Some(Action::SkipCurrent),
                crossterm::event::KeyCode::Char('s') => return Some(Action::Sync),
                crossterm::event::KeyCode::Char('d') => return Some(Action::DeleteMode),
                crossterm::event::KeyCode::Char('w') => return Some(Action::WipeMode),
                crossterm::event::KeyCode::Char('c') => return Some(Action::SetupCdMode),
                crossterm::event::KeyCode::Char('k') => return Some(Action::ConnectionsMode),
                crossterm::event::KeyCode::Char('v') => return Some(Action::ToggleVerbose),
                crossterm::event::KeyCode::Char('q') => return Some(Action::Quit),
                crossterm::event::KeyCode::Char('?') => {
                    self.show_help = true;
                    self.help_scroll = 0;
                    return None;
                }
                _ => return None,
            }
        }
        
        // From this point on, we're in Idle mode with no modal questions
        
        // Detected pane: navigate detected devices and add them
        if self.active_pane == ActivePane::Detected && !in_modal {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    if !self.detected.is_empty() {
                        return Some(Action::PendingSelectPrev);
                    }
                }
                crossterm::event::KeyCode::Down => {
                    if !self.detected.is_empty() {
                        return Some(Action::PendingSelectNext);
                    }
                }
                crossterm::event::KeyCode::Char('a') => {
                    return self.detected_selected.map(|_| Action::Add);
                }
                crossterm::event::KeyCode::Char('n') => {
                    return self.detected_selected.map(|_| Action::Ignore);
                }
                _ => {}
            }
        }
        
        // Added pane: navigate added devices and perform actions
        if self.active_pane == ActivePane::Added && !in_modal {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    if !self.added.is_empty() {
                        return Some(Action::AddedSelectPrev);
                    }
                }
                crossterm::event::KeyCode::Down => {
                    if !self.added.is_empty() {
                        return Some(Action::AddedSelectNext);
                    }
                }
                crossterm::event::KeyCode::Char('i') => {
                    return self.added_selected.map(|_| Action::InstallKeyToSelected);
                }
                crossterm::event::KeyCode::Char('r') => {
                    return self.added_selected.map(|_| Action::RemoveKeyFromSelected);
                }
                crossterm::event::KeyCode::Char('d') => {
                    return self.added_selected.map(|_| Action::DeleteSelected);
                }
                crossterm::event::KeyCode::Char('s') => {
                    return Some(Action::Sync);
                }
                crossterm::event::KeyCode::Char('w') => {
                    return Some(Action::WipeMode);
                }
                crossterm::event::KeyCode::Char('c') => {
                    return Some(Action::SetupCdMode);
                }
                _ => {}
            }
        }
        
        // Logs pane: scroll and toggle verbose
        if self.active_pane == ActivePane::Logs && !in_modal {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    self.log_scroll = self.log_scroll.saturating_add(1);
                    return None;
                }
                crossterm::event::KeyCode::Down => {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                    return None;
                }
                crossterm::event::KeyCode::PageUp => {
                    self.log_scroll = self.log_scroll.saturating_add(10);
                    return None;
                }
                crossterm::event::KeyCode::PageDown => {
                    self.log_scroll = self.log_scroll.saturating_sub(10);
                    return None;
                }
                crossterm::event::KeyCode::Char('v') => {
                    return Some(Action::ToggleVerbose);
                }
                _ => {}
            }
        }
        
        // Pane navigation by letters only (NO TAB)
        // Esc returns to no active pane (MainMenu)
        // Only handle pane navigation if we're not currently in that pane
        // This allows pane-specific actions (like 'a' to add in Detected pane) to take precedence
        if !in_modal {
            match key.code {
                crossterm::event::KeyCode::Char('d') => {
                    // Only switch to Detected if we're not already there
                    if self.active_pane != ActivePane::Detected {
                        self.active_pane = ActivePane::Detected;
                        self.log_scroll = 0;
                        return None;
                    }
                }
                crossterm::event::KeyCode::Char('a') => {
                    // Only switch to Added if we're not already there
                    if self.active_pane != ActivePane::Added {
                        self.active_pane = ActivePane::Added;
                        self.log_scroll = 0;
                        return None;
                    }
                }
                crossterm::event::KeyCode::Char('l') => {
                    // Only switch to Logs if we're not already there
                    if self.active_pane != ActivePane::Logs {
                        self.active_pane = ActivePane::Logs;
                        self.log_scroll = 0;
                        return None;
                    }
                }
                _ => {}
            }
        }
        
        // Global actions (available from any pane)
        match key.code {
            crossterm::event::KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                self.help_scroll = 0;
                None
            }
            crossterm::event::KeyCode::Char('q') => Some(Action::Quit),
            _ => None,
        }
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
            show_help,
            help_scroll,
            verbose,
            active_pane,
            flashes,
            pulses,
            key_flash,
            ..
        } = self;
        let mut cursor = None;
        terminal.draw(|frame| {
            // Calculate the height needed for the question block
            let question_ctx = QuestionCtx {
                input,
                pending: detected.len(),
                help: *show_help,
                verbose: *verbose,
                active_pane: *active_pane,
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
                    detected,
                    *detected_selected,
                    added,
                    *added_selected,
                    log,
                    *log_scroll,
                    *verbose,
                    flashes,
                    pulses,
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
