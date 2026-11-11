//! Plain line-based fallback UI for non-terminal output.
//!
//! This module provides a simple text-based interface that works when
//! stdout is not a terminal (e.g., when output is piped to another command).

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::types::{Action, Question};

/// Plain line-based fallback for non-terminal stdout, mirroring the
/// interface the tool had before the full-screen ui.
pub struct PlainUi {
    rx: Receiver<String>,
    question: Question,
}

impl PlainUi {
    /// Create a new PlainUi instance.
    pub fn new() -> Self {
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

    /// Set the current question. Prints the prompt when the question changes.
    pub fn set_question(&mut self, question: Question) {
        if question != self.question {
            self.question = question;
            print!("{} ", self.prompt());
            let _ = io::stdout().flush();
        }
    }

    /// Prompt text for the current question.
    pub fn prompt(&self) -> String {
        match &self.question {
            Question::Idle => {
                "[q=quit, s=sync git, d=delete, w=wipe, c=setup-cd, v=verbose, h=help]:".to_string()
            }
            Question::AddDevice { name, ip } => {
                format!("Add {name} ({ip}) to config? [y/n/q/s]:")
            }
            Question::TrustKey { name } => {
                format!("Trust the new signing key for {name}? [y/n]:")
            }
            Question::Rename { name, suggested } => {
                format!("New name for {name} (empty = {suggested}):")
            }
            Question::Delete { name } => format!("Delete {name}? [y/n]:"),
            Question::Wipe { count } => {
                format!(
                    "Wipe the ENTIRE configuration ({count} device block(s))? [y/n]:"
                )
            }
            Question::SetupCd { file } => {
                format!("Install 'regroup-cd' in ~/{file}? [y/n]:")
            }
            Question::InstallKeySelectDevice { devices } => {
                format!(
                    "Select target device for key installation ({} available):",
                    devices.len()
                )
            }
            Question::InstallKeySelectUser { device, users } => {
                format!(
                    "Select user on {} for key installation ({} available):",
                    device, users.len()
                )
            }
            Question::InstallKeyConfirm {
                from_user,
                from_device,
                to_user,
                to_device,
            } => {
                format!(
                    "Install SSH key from {}@{} to {}@{}? [y/n]:",
                    from_user, from_device, to_user, to_device
                )
            }
            Question::RemoveKeySelectConnection { connections } => {
                format!("Select connection to remove ({} available):", connections.len())
            }
            Question::RemoveKeyConfirm { connection_id, .. } => {
                format!("Remove SSH key {connection_id}? [y/n]:")
            }
        }
    }

    /// Wait up to `timeout` for a user action.
    pub fn poll_action(&mut self, timeout: Duration) -> Option<Action> {
        let line = self.rx.recv_timeout(timeout).ok()?;
        let line = line.trim();
        self.translate_action(line)
    }

    /// Translate an input line into an Action based on the current question.
    fn translate_action(&self, line: &str) -> Option<Action> {
        match (&self.question, line) {
            (Question::Idle, "q") => Some(Action::Quit),
            (Question::Idle, "s") => Some(Action::Sync),
            (Question::Idle, "d") => Some(Action::DeleteMode),
            (Question::Idle, "w") => Some(Action::WipeMode),
            (Question::Idle, "c") => Some(Action::SetupCdMode),
            (Question::Idle, "k") => Some(Action::ConnectionsMode),
            (Question::Idle, "v") => Some(Action::ToggleVerbose),
            (Question::Idle, "h" | "?") => Some(Action::Help),
            (Question::Idle, _) => None,
            (Question::AddDevice { .. }, "y") => Some(Action::AddCurrent),
            (Question::AddDevice { .. }, "n") => Some(Action::SkipCurrent),
            (Question::AddDevice { .. }, "s") => Some(Action::Sync),
            (Question::AddDevice { .. }, "d") => Some(Action::DeleteMode),
            (Question::AddDevice { .. }, "w") => Some(Action::WipeMode),
            (Question::AddDevice { .. }, "c") => Some(Action::SetupCdMode),
            (Question::AddDevice { .. }, "k") => Some(Action::ConnectionsMode),
            (Question::AddDevice { .. }, "q") => Some(Action::Quit),
            (Question::AddDevice { .. }, _) => Some(Action::SkipCurrent),
            (Question::TrustKey { .. }, "y") => Some(Action::TrustKey),
            (Question::TrustKey { .. }, "n") => Some(Action::RejectKey),
            (Question::TrustKey { .. }, _) => Some(Action::RejectKey),
            (Question::Rename { suggested, .. }, "") => Some(Action::Rename(suggested.clone())),
            (Question::Rename { .. }, "q") => Some(Action::SkipRename),
            (Question::Rename { .. }, name) => Some(Action::Rename(name.to_string())),
            (Question::Delete { .. }, "y") => Some(Action::DeleteConfirm),
            (Question::Delete { .. }, "n") => Some(Action::DeleteCancel),
            (Question::Delete { .. }, _) => Some(Action::DeleteCancel),
            (Question::Wipe { .. }, "y") => Some(Action::WipeConfirm),
            (Question::Wipe { .. }, "n") => Some(Action::WipeCancel),
            (Question::Wipe { .. }, _) => Some(Action::WipeCancel),
            (Question::SetupCd { .. }, "y") => Some(Action::SetupCdConfirm),
            (Question::SetupCd { .. }, "n") => Some(Action::SetupCdCancel),
            (Question::SetupCd { .. }, _) => Some(Action::SetupCdCancel),
            // Key installation/removal actions
            (Question::InstallKeySelectDevice { .. }, "q") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectDevice { .. }, "n") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectDevice { .. }, _) => Some(Action::InstallKeyMode),
            (Question::InstallKeySelectUser { .. }, "q") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectUser { .. }, "n") => Some(Action::InstallKeyCancel),
            (Question::InstallKeySelectUser { .. }, _device) => {
                // This would require more context to handle properly
                Some(Action::InstallKeyMode)
            }
            (Question::InstallKeyConfirm { .. }, "y") => Some(Action::InstallKeyConfirm),
            (Question::InstallKeyConfirm { .. }, "n") => Some(Action::InstallKeyCancel),
            (Question::InstallKeyConfirm { .. }, _) => Some(Action::InstallKeyCancel),
            (Question::RemoveKeySelectConnection { .. }, "q") => Some(Action::RemoveKeyCancel),
            (Question::RemoveKeySelectConnection { .. }, "n") => Some(Action::RemoveKeyCancel),
            (Question::RemoveKeySelectConnection { .. }, _) => Some(Action::RemoveKeyMode),
            (Question::RemoveKeyConfirm { .. }, "y") => Some(Action::RemoveKeyConfirm),
            (Question::RemoveKeyConfirm { .. }, "n") => Some(Action::RemoveKeyCancel),
            (Question::RemoveKeyConfirm { .. }, _) => Some(Action::RemoveKeyCancel),
        }
    }
}
