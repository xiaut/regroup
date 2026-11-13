//! Plain line-based fallback UI for non-terminal output.
//!
//! This module provides a simple text-based interface that works when
//! stdout is not a terminal (e.g., when output is piped to another command).

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crossterm::event::KeyCode;

use super::types::{Action, Effect, KeyBinding, Question};

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

    /// Translate an input line into an Action by looking the line up in
    /// the current question's binding table — the same table the prompt
    /// is printed from, so an advertised key always works. A line maps
    /// to a key: empty means Enter, a single character means that char.
    fn translate_action(&self, line: &str) -> Option<Action> {
        let line = line.trim();
        let table: Vec<&KeyBinding> = self.question.def().bindings.iter().collect();
        let code = if line.is_empty() {
            Some(KeyCode::Enter)
        } else {
            let mut chars = line.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(KeyCode::Char(c)),
                _ => None,
            }
        };
        if let Some(code) = code
            && let Some(binding) = table.iter().find(|b| b.key == code)
        {
            return match &binding.effect {
                Effect::Action(action) => Some(action.clone()),
                Effect::SubmitInput => match &self.question {
                    // An empty line accepts the suggested name; an empty
                    // user name cancels.
                    Question::Rename(_, _, suggested) => Some(Action::Rename(if line.is_empty() {
                        suggested.clone()
                    } else {
                        line.to_string()
                    })),
                    Question::AddTempUser(_, _) => {
                        if line.is_empty() {
                            Some(Action::CancelTempUser)
                        } else {
                            Some(Action::TempUserName(line.to_string()))
                        }
                    }
                    _ => None,
                },
                Effect::Internal => None,
            };
        }
        // Line-based fallbacks: a textual answer for the input questions,
        // the question's default answer for the confirm questions.
        match &self.question {
            Question::Rename(_, _, _) if !line.is_empty() => Some(Action::Rename(line.to_string())),
            Question::AddTempUser(_, _) if !line.is_empty() => {
                Some(Action::TempUserName(line.to_string()))
            }
            Question::AddDevice(..) => Some(Action::SkipCurrent),
            Question::TrustKey(..) => Some(Action::RejectKey),
            Question::Delete(..) => Some(Action::DeleteCancel),
            Question::Wipe(..) => Some(Action::WipeCancel),
            Question::SetupCd(..) => Some(Action::SetupCdCancel),
            _ => None,
        }
    }
}
