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
            question: Question::idle(),
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
            Question::Idle(def) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}={}", b.label, b.description))
                    .collect();
                format!("[{}]:", hints.join(", "))
            }
            Question::AddDevice(def, name, ip) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}", b.label))
                    .collect();
                format!("Add {name} ({ip}) to config? [{}]:", hints.join("/"))
            }
            Question::TrustKey(def, name) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}", b.label))
                    .collect();
                format!("Trust the new signing key for {name}? [{}]:", hints.join("/"))
            }
            Question::Rename(_, name, suggested) => {
                format!("New name for {name} (empty = {suggested}):")
            }
            Question::Delete(def, name) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}", b.label))
                    .collect();
                format!("Delete {name}? [{}]:", hints.join("/"))
            }
            Question::Wipe(def, count) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}", b.label))
                    .collect();
                format!("Wipe the ENTIRE configuration ({count} device block(s))? [{}]:", hints.join("/"))
            }
            Question::SetupCd(def, file) => {
                let hints: Vec<String> = def.bindings.iter()
                    .map(|b| format!("{}", b.label))
                    .collect();
                format!("Install 'regroup-cd' in ~/{file}? [{}]:", hints.join("/"))
            }
            Question::AddTempUser(_, device) => {
                format!("Temporary user on {device} (empty = cancel):")
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
            (Question::Idle(_), "q") => Some(Action::Quit),
            (Question::Idle(_), "s") => Some(Action::Sync),
            (Question::Idle(_), "d") => Some(Action::DeleteMode),
            (Question::Idle(_), "w") => Some(Action::WipeMode),
            (Question::Idle(_), "c") => Some(Action::SetupCdMode),
            (Question::Idle(_), "k") => Some(Action::ConnectionsMode),
            (Question::Idle(_), "v") => Some(Action::ToggleVerbose),
            (Question::Idle(_), "h" | "?") => Some(Action::Help),
            (Question::Idle(_), _) => None,
            (Question::AddDevice(_, _, _), "y") => Some(Action::AddCurrent),
            (Question::AddDevice(_, _, _), "n") => Some(Action::SkipCurrent),
            (Question::AddDevice(_, _, _), "s") => Some(Action::Sync),
            (Question::AddDevice(_, _, _), "d") => Some(Action::DeleteMode),
            (Question::AddDevice(_, _, _), "w") => Some(Action::WipeMode),
            (Question::AddDevice(_, _, _), "c") => Some(Action::SetupCdMode),
            (Question::AddDevice(_, _, _), "k") => Some(Action::ConnectionsMode),
            (Question::AddDevice(_, _, _), "q") => Some(Action::Quit),
            (Question::AddDevice(_, _, _), _) => Some(Action::SkipCurrent),
            (Question::TrustKey(_, _), "y") => Some(Action::TrustKey),
            (Question::TrustKey(_, _), "n") => Some(Action::RejectKey),
            (Question::TrustKey(_, _), _) => Some(Action::RejectKey),
            (Question::Rename(_, _, suggested), "") => Some(Action::Rename(suggested.clone())),
            (Question::Rename(_, _, _), "q") => Some(Action::SkipRename),
            (Question::Rename(_, _, _), name) => Some(Action::Rename(name.to_string())),
            (Question::Delete(_, _), "y") => Some(Action::DeleteConfirm),
            (Question::Delete(_, _), "n") => Some(Action::DeleteCancel),
            (Question::Delete(_, _), _) => Some(Action::DeleteCancel),
            (Question::Wipe(_, _), "y") => Some(Action::WipeConfirm),
            (Question::Wipe(_, _), "n") => Some(Action::WipeCancel),
            (Question::Wipe(_, _), _) => Some(Action::WipeCancel),
            (Question::SetupCd(_, _), "y") => Some(Action::SetupCdConfirm),
            (Question::SetupCd(_, _), "n") => Some(Action::SetupCdCancel),
            (Question::SetupCd(_, _), _) => Some(Action::SetupCdCancel),
            (Question::AddTempUser(_, _), "") => Some(Action::CancelTempUser),
            (Question::AddTempUser(_, _), name) => Some(Action::TempUserName(name.to_string())),
        }
    }
}
