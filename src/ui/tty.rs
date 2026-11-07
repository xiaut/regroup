//! Full-screen terminal UI implementation using ratatui.
//!
//! This module provides the TtyUi struct which handles the full-screen TUI
//! interface, including device panels, activity log, and question display.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::DefaultTerminal;


use super::render::{
    key_label, render_help, render_log, render_machines, render_question,
    KeyFlash, PanelTheme, QuestionCtx, RowFlash,
};
use super::types::{ActivePane, Action, Device, Question, UiEvent};
use crate::cli::usage_text;

/// Full-screen ratatui interface.
pub struct TtyUi {
    pub terminal: DefaultTerminal,
    pub log_rx: Receiver<UiEvent>,
    pub log: Vec<String>,
    /// Lines scrolled up from the bottom of the activity pane; 0 follows it.
    pub scroll: u16,
    pub question: Question,
    /// Text typed for the rename question.
    pub input: String,
    /// Machines announced but not accepted yet (Detected panel).
    pub detected: Vec<Device>,
    /// Machines in the configuration (Added panel).
    pub added: Vec<Device>,
    /// Row of the Detected panel highlighted by the selection.
    pub selected: Option<usize>,
    /// Row of the Added panel highlighted by the selection.
    pub added_selected: Option<usize>,
    /// Row of the Added panel highlighted in delete mode.
    pub delete_selected: Option<usize>,
    /// Which pane is currently active for navigation and actions.
    pub active_pane: ActivePane,
    /// Whether the help panel is shown (key '?'): it replaces the machine
    /// panels and the activity pane; the question block stays interactive.
    pub show_help: bool,
    /// Lines scrolled up from the bottom of the help panel; 0 follows it.
    pub help_scroll: u16,
    /// Verbose mode: reported in the activity pane title and the status
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
    /// and resets the activity scroll whenever the question changes.
    pub fn set_question(&mut self, question: Question) {
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
        
        // Handle pane switching with Tab
        if !in_modal && matches!(key.code, crossterm::event::KeyCode::Tab) {
            self.active_pane = match self.active_pane {
                ActivePane::Detected => ActivePane::Added,
                ActivePane::Added => ActivePane::Detected,
            };
            // Reset selection when switching panes
            self.selected = None;
            self.added_selected = None;
            return None;
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
        // Handle pane-specific actions
        
        // Detected pane: navigate detected devices and add them
        if self.active_pane == ActivePane::Detected {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    if !self.detected.is_empty() {
                        self.selected = Some(self.selected.map_or(0, |s| s.saturating_sub(1)));
                        return None;
                    }
                    self.scroll = self.scroll.saturating_add(1);
                    return None;
                }
                crossterm::event::KeyCode::Down => {
                    if !self.detected.is_empty() {
                        self.selected = Some(self.selected.map_or(0, |s| {
                            (s + 1).min(self.detected.len() - 1)
                        }));
                        return None;
                    }
                    self.scroll = self.scroll.saturating_sub(1);
                    return None;
                }
                crossterm::event::KeyCode::PageUp => {
                    self.scroll = self.scroll.saturating_add(10);
                    return None;
                }
                crossterm::event::KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_sub(10);
                    return None;
                }
                crossterm::event::KeyCode::Enter => {
                    return self.selected.map(|_| Action::Add);
                }
                crossterm::event::KeyCode::Char('a') => {
                    return self.selected.map(|_| Action::Add);
                }
                crossterm::event::KeyCode::Char('n') | crossterm::event::KeyCode::Char('x') => {
                    return self.selected.map(|_| Action::Ignore);
                }
                _ => {}
            }
        }
        
        // Added pane: navigate added devices and perform actions on them
        if self.active_pane == ActivePane::Added {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    if !self.added.is_empty() {
                        self.added_selected = Some(self.added_selected.map_or(0, |s| s.saturating_sub(1)));
                        return None;
                    }
                    self.scroll = self.scroll.saturating_add(1);
                    return None;
                }
                crossterm::event::KeyCode::Down => {
                    if !self.added.is_empty() {
                        self.added_selected = Some(self.added_selected.map_or(0, |s| {
                            (s + 1).min(self.added.len() - 1)
                        }));
                        return None;
                    }
                    self.scroll = self.scroll.saturating_sub(1);
                    return None;
                }
                crossterm::event::KeyCode::PageUp => {
                    self.scroll = self.scroll.saturating_add(10);
                    return None;
                }
                crossterm::event::KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_sub(10);
                    return None;
                }
                crossterm::event::KeyCode::Enter => {
                    return self.added_selected.map(|_| Action::InstallKeyToSelected);
                }
                crossterm::event::KeyCode::Char('i') => {
                    return self.added_selected.map(|_| Action::InstallKeyToSelected);
                }
                crossterm::event::KeyCode::Char('d') => {
                    return self.added_selected.map(|_| Action::DeleteSelected);
                }
                crossterm::event::KeyCode::Char('r') => {
                    return self.added_selected.map(|_| Action::RemoveKeyFromSelected);
                }
                _ => {}
            }
        }
        
        // Global actions (available regardless of active pane)
        match key.code {
            crossterm::event::KeyCode::Char('q') => Some(Action::Quit),
            crossterm::event::KeyCode::Char('s') => Some(Action::Sync),
            crossterm::event::KeyCode::Char('w') => Some(Action::WipeMode),
            crossterm::event::KeyCode::Char('c') => Some(Action::SetupCdMode),
            crossterm::event::KeyCode::Char('k') => Some(Action::ConnectionsMode),
            crossterm::event::KeyCode::Char('v') => Some(Action::ToggleVerbose),
            crossterm::event::KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                self.help_scroll = 0;
                None
            }
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
                let max_scroll = usage_text()
                    .lines()
                    .count()
                    .saturating_sub(visible) as u16;
                let help_scroll_bounded = (*help_scroll).min(max_scroll);
                render_help(frame, help_area, help_scroll_bounded);
            } else {
                let narrow = frame.area().width < super::render::MIN_WIDTH_FOR_SIDE_BY_SIDE;
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
