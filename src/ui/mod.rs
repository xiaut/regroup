//! Terminal user interface for interactive mode.
//!
//! Two backends behind the same interface:
//! - a full-screen ratatui interface when stdout is a terminal: two device
//!   panels (detected devices on the left, added devices on the right when
//!   the screen is wide enough; otherwise detected on top, added below),
//!   an activity log for general messages, and the current question as the
//!   bottom block — the question can never be pushed away by notifications;
//!   key '?' replaces the device panels and the activity pane with the CLI
//!   help, keeping the bottom block;
//! - a plain line-based fallback otherwise (piped output): prompts are
//!   printed as they change and answers are read as input lines.
//!
//! All modules display messages through [`log`] (general messages, shown in
//! the activity pane) or [`log_device`] (device-specific messages, shown
//! as a temporary animated status on the device's row). There is a single
//! log stream: the UI channel, which the full-screen ui shows in full in its
//! Logs pane. Without a UI channel (startup, plain mode) the stream is
//! stdout. During a temporary CLI pass from the TUI (direct output mode,
//! see [`DirectOutputGuard`]) the stream is shown in both places: every
//! message is printed to the CLI and recorded for the Logs pane, and
//! external-command output is forwarded live to the CLI by
//! [`run_logged`] while being recorded the same way.

pub mod interface;
pub mod keys;
pub mod logging;
pub mod plain;
pub mod render;
pub mod subprocess;
pub mod tty;
pub mod types;

// Re-export the main UI types and functions for convenience
pub use interface::Ui;
pub use logging::{log, log_device, pulse_device, restore, DirectOutputGuard};
pub use subprocess::run_logged;
pub use types::{Action, Level, Question};
