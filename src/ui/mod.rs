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
//! as a temporary animated status on the device's row). Before the
//! full-screen ui runs (startup, plain mode) both go to stdout; afterwards
//! they are buffered in a channel.

pub mod interface;
pub mod logging;
pub mod plain;
pub mod render;
pub mod tty;
pub mod types;

// Re-export the main UI types and functions for convenience
pub use interface::Ui;
pub use logging::{log, log_device, pulse_device, restore, set_direct_output_mode};
pub use types::{Action, Level, Question};
