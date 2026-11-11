//! UI logging infrastructure.
//!
//! Provides functions for displaying messages through the UI, either in the
//! activity pane (full-screen mode) or on stdout (plain mode).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::ssh::keys::SshPublicKey;

use super::types::{Level, UiEvent};

/// Maximum number of activity messages kept for display.
pub const LOG_CAP: usize = 1000;

/// Global atomic flag to indicate whether logs should go directly to stdout.
/// This is used when temporarily switching from TUI to CLI mode.
/// When true, log() and log_device() print directly to stdout instead of
/// sending to the channel.
static DIRECT_OUTPUT_MODE: AtomicBool = AtomicBool::new(false);

/// Check if we're in direct output mode.
pub fn is_direct_output_mode() -> bool {
    DIRECT_OUTPUT_MODE.load(Ordering::Relaxed)
}

/// Set direct output mode on or off.
pub fn set_direct_output_mode(enabled: bool) {
    DIRECT_OUTPUT_MODE.store(enabled, Ordering::Relaxed);
}

/// How long a device's row keeps its animated status.
pub const FLASH_MS: u64 = 2500;
/// Security flashes last longer: the user must not miss them.
pub const SECURITY_FLASH_MS: u64 = 5000;
/// Half-period of the blink of warning and security flashes.
pub const BLINK_MS: u64 = 250;
/// How long the pressed key stays highlighted in the hint line.
pub const KEY_FLASH_MS: u64 = 700;
/// How long a device's row animates after a beacon is received from it.
pub const PULSE_MS: u64 = 900;

/// Sender side of the log channel; unset until [`crate::ui::Ui::start`] creates it.
pub static LOG_TX: OnceLock<Sender<UiEvent>> = OnceLock::new();

/// Display a general message: in the activity pane once the full-screen ui
/// runs, on stdout otherwise.
pub fn log(msg: &str) {
    if is_direct_output_mode() {
        println!("{msg}");
    } else {
        match LOG_TX.get() {
            Some(tx) => {
                let _ = tx.send(UiEvent::Global(msg.to_string()));
            }
            None => println!("{msg}"),
        }
    }
}

/// Display a device-specific message: animated status on the device's
/// row in the full-screen ui, `detail` (or `name: msg`) on stdout otherwise.
/// Security messages are additionally kept in the activity pane.
pub fn log_device(
    key: &SshPublicKey,
    name: &str,
    level: Level,
    msg: &str,
    detail: Option<&str>,
) {
    if is_direct_output_mode() {
        match detail {
            Some(detail) => println!("{detail}"),
            None => println!("{name}: {msg}"),
        }
    } else {
        match LOG_TX.get() {
            Some(tx) => {
                let _ = tx.send(UiEvent::UiDevice {
                    key: key.as_str().to_string(),
                    name: name.to_string(),
                    level,
                    msg: msg.to_string(),
                    detail: detail.map(str::to_string),
                });
            }
            None => match detail {
                Some(detail) => println!("{detail}"),
                None => println!("{name}: {msg}"),
            },
        }
    }
}

/// Pulse a device's row: the row lights up and fades out for a short
/// while, showing the user that a beacon was received from it (even a
/// silent one: known device, unchanged IP). Full-screen ui only.
pub fn pulse_device(key: &SshPublicKey) {
    if let Some(tx) = LOG_TX.get() {
        let _ = tx.send(UiEvent::Pulse {
            key: key.as_str().to_string(),
        });
    }
}

/// Best-effort terminal restoration (raw mode off, leave alternate screen),
/// e.g. before `process::exit` from a code path that cannot run destructors.
pub fn restore() {
    ratatui::restore();
}

/// How long the animated status of a message of this level lasts.
pub fn flash_duration(level: Level) -> Duration {
    Duration::from_millis(match level {
        Level::Security => SECURITY_FLASH_MS,
        _ => FLASH_MS,
    })
}
