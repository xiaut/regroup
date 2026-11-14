//! Unified UI interface that abstracts over Tty and Plain backends.
//!
//! This module provides the `Ui` enum that delegates to either the full-screen
//! TUI or the plain text fallback, depending on whether stdout is a terminal.

use std::io::{self, IsTerminal, Write};
use std::sync::mpsc;
use std::time::Duration;

use super::logging::LOG_TX;
use super::plain::PlainUi;
use super::tty::TtyUi;
use super::types::{Action, Navigation, Question, UiDevice, UiUser};
use super::types::ActivePane;

/// The interface used by the interactive loop.
pub enum Ui {
    Tty(Box<TtyUi>),
    Plain(PlainUi),
}

impl Ui {
    /// Start the interface: full-screen when stdout is a terminal, plain
    /// otherwise. When `force_cli` is true, always use plain mode.
    pub fn start(force_cli: bool) -> Ui {
        if !force_cli && io::stdout().is_terminal() {
            let terminal = ratatui::init();
            let (tx, log_rx) = mpsc::channel();
            let _ = LOG_TX.set(tx);
            Ui::Tty(Box::new(TtyUi {
                terminal,
                log_rx,
                log: Vec::new(),
                log_scroll: 0,
                question: Question::idle(),
                input: String::new(),
                detected: Vec::new(),
                added: Vec::new(),
                detected_selected: None,
                added_selected: None,
                delete_selected: None,
                users: Vec::new(),
                users_selected: None,
                navigation: Navigation::overview(),
                active_pane: ActivePane::main_menu(),
                show_help: false,
                help_scroll: 0,
                flashes: std::collections::HashMap::new(),
                pulses: std::collections::HashMap::new(),
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

    /// Update the device panels (ignored in plain mode).
    pub fn set_devices(&mut self, detected: Vec<UiDevice>, added: Vec<UiDevice>) {
        if let Ui::Tty(ui) = self {
            ui.detected = detected;
            ui.added = added;
        }
    }

    /// Highlight the selected row of the Detected panel.
    pub fn set_selected(&mut self, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.detected_selected = selected;
        }
    }

    /// Highlight the selected row of the Added panel (delete mode).
    pub fn set_delete_selected(&mut self, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.delete_selected = selected;
        }
    }

    /// Highlight the selected row of the Added panel (normal mode).
    pub fn set_added_selected(&mut self, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.added_selected = selected;
        }
    }

    /// Update the Users panel: the users of the machine selected in the
    /// Added panel, and the selected row (ignored in plain mode).
    pub fn set_users(&mut self, users: Vec<UiUser>, selected: Option<usize>) {
        if let Ui::Tty(ui) = self {
            ui.users = users;
            ui.users_selected = selected;
        }
    }

    /// Set the currently active pane (derived from navigation).
    pub fn set_active_pane(&mut self, pane: ActivePane) {
        if let Ui::Tty(ui) = self {
            ui.active_pane = pane;
        }
    }

    /// Set the current navigation state.
    pub fn set_navigation(&mut self, navigation: Navigation) {
        if let Ui::Tty(ui) = self {
            ui.navigation = navigation;
        }
    }

    /// Get the current navigation state (Tty UI only).
    pub fn navigation(&self) -> Option<Navigation> {
        if let Ui::Tty(ui) = self {
            Some(ui.navigation)
        } else {
            None
        }
    }

    /// Get the current active pane (Tty UI only).
    pub fn active_pane(&self) -> Option<ActivePane> {
        if let Ui::Tty(ui) = self {
            Some(ui.active_pane)
        } else {
            None
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

    /// Execute a function with a clean terminal (restored to normal mode),
    /// then reinitialize the TUI terminal afterwards. This is useful when
    /// running CLI commands from within the TUI that need a clean terminal.
    pub fn with_clean_terminal<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        // Check if we're in Tty mode
        match self {
            Ui::Tty(ui_box) => {
                // Restore the terminal (leave alternate screen, disable raw mode)
                ratatui::restore();

                // Enable direct output mode so logs are printed to stdout immediately
                // instead of being queued in the channel
                crate::ui::set_direct_output_mode(true);
                
                // Execute the function
                let result = f();

                // Disable direct output mode
                crate::ui::set_direct_output_mode(false);

                // Pause to let the user read the output
                println!("\nPress Enter to return to the TUI...");
                let _ = io::stdout().flush();
                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);

                // Reinitialize the terminal
                // Note: ratatui::init() panics on failure, which is acceptable here
                // as it indicates a terminal initialization error
                ui_box.terminal = ratatui::init();

                result
            }
            Ui::Plain(_) => {
                // In plain mode, just execute the function
                // Direct output mode is already false in plain mode
                f()
            }
        }
    }
}
