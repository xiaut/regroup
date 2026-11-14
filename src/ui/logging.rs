//! UI logging infrastructure.
//!
//! There is a single log stream: the UI channel, which the full-screen TUI
//! shows in full in its Logs pane. [`log`] and [`log_device`] are its entry
//! points: they always record into the stream and additionally print to
//! stdout when there is no UI channel (startup, plain mode) or during a
//! temporary CLI pass from the TUI (direct output mode) — so the CLI shows
//! exactly the lines emitted during passes. [`record`] adds an event to the
//! stream without printing, for output already shown live in the CLI during
//! a pass (see [`crate::ui::subprocess::run_logged`]).

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
/// When true, log() and log_device() print directly to stdout (so the line
/// appears in the CLI during the pass) in addition to being sent to the
/// channel (so it still lands in the activity pane when the TUI resumes).
static DIRECT_OUTPUT_MODE: AtomicBool = AtomicBool::new(false);

/// Check if we're in direct output mode.
pub fn is_direct_output_mode() -> bool {
    DIRECT_OUTPUT_MODE.load(Ordering::Relaxed)
}

/// RAII guard that enables direct output mode for its lifetime and restores
/// the previous value on drop (even on panic).
///
/// While held, [`log`] and [`log_device`] also print to stdout (in addition
/// to the UI channel): everything logged during a temporary CLI pass from
/// the TUI appears in the CLI immediately, and in the activity pane when the
/// TUI resumes. [`crate::ui::Ui::with_clean_terminal`] holds this guard for
/// the whole CLI-visible window — the operation itself and the "press Enter
/// to return" pause — until the TUI is re-initialized.
pub struct DirectOutputGuard {
    previous: bool,
}

impl DirectOutputGuard {
    pub fn new() -> Self {
        let previous = DIRECT_OUTPUT_MODE.swap(true, Ordering::Relaxed);
        Self { previous }
    }
}

impl Drop for DirectOutputGuard {
    fn drop(&mut self) {
        DIRECT_OUTPUT_MODE.store(self.previous, Ordering::Relaxed);
    }
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

/// Record an event in the one log stream (the UI channel when the
/// full-screen ui runs) without printing it. Used for output that is
/// already shown live in the CLI during a temporary CLI pass: it appears in
/// the CLI now and in the TUI Logs pane when the TUI resumes.
pub fn record(event: UiEvent) {
    if let Some(tx) = LOG_TX.get() {
        let _ = tx.send(event);
    }
}

/// Display a general message: in the activity pane once the full-screen ui
/// runs, on stdout otherwise. During a temporary CLI pass (direct output
/// mode) the message is also printed to stdout — it appears in the CLI —
/// while still being recorded in the stream for the activity pane.
pub fn log(msg: &str) {
    record(UiEvent::Global(msg.to_string()));
    if is_direct_output_mode() || LOG_TX.get().is_none() {
        println!("{msg}");
    }
}

/// Display a device-specific message: animated status on the device's
/// row in the full-screen ui, `detail` (or `name: msg`) on stdout otherwise.
/// Security messages are additionally kept in the activity pane.
/// During a temporary CLI pass (direct output mode) the message is printed
/// to stdout — it appears in the CLI — and still sent to the channel, so
/// the row still flashes when the TUI resumes.
pub fn log_device(
    key: &SshPublicKey,
    name: &str,
    level: Level,
    msg: &str,
    detail: Option<&str>,
) {
    record(UiEvent::UiDevice {
        key: key.as_str().to_string(),
        name: name.to_string(),
        level,
        msg: msg.to_string(),
        detail: detail.map(str::to_string),
    });
    if is_direct_output_mode() || LOG_TX.get().is_none() {
        match detail {
            Some(detail) => println!("{detail}"),
            None => println!("{name}: {msg}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    // Single test touching the global flag: cargo runs tests in parallel
    // threads within one process, so two tests toggling DIRECT_OUTPUT_MODE
    // could race each other. Other tests only read the flag via log(),
    // which behaves the same either way while LOG_TX is unset.
    #[test]
    fn direct_output_mode_tees_logs_to_stdout_and_the_ui_channel() {
        // Install a channel as the UI sink, like Ui::start does for the Tty backend.
        let (tx, rx) = mpsc::channel();
        let _ = LOG_TX.set(tx);

        // Guard on: direct output mode is active until drop, even if the
        // guarded scope panics (Drop restores the previous value).
        let before = is_direct_output_mode();
        {
            let _guard = DirectOutputGuard::new();
            assert!(is_direct_output_mode());

            // During a temporary CLI pass the line is printed to stdout
            // (it appears in the CLI) AND forwarded to the UI channel, so
            // it still lands in the activity pane when the TUI resumes.
            log("to-cli");
            log_device(
                &SshPublicKey::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFake"),
                "dev",
                Level::Info,
                "msg",
                Some("detail"),
            );

            // External commands run through run_logged belong to the same
            // stream: in direct mode their output is shown live in the CLI
            // (pumped to stdout here) and recorded into the UI channel.
            let mut echo = std::process::Command::new("echo");
            echo.arg("pass-echo");
            let out = crate::ui::run_logged(echo).expect("run echo");
            assert!(out.status.success());
            assert!(String::from_utf8_lossy(&out.stdout).contains("pass-echo"));
        }
        assert_eq!(is_direct_output_mode(), before);

        // Outside the pass, logs go to the UI channel (TUI pane) only, and
        // run_logged captures the command output and logs it.
        log("to-pane");
        let mut echo = std::process::Command::new("echo");
        echo.arg("normal-echo");
        let out = crate::ui::run_logged(echo).expect("run echo");
        assert!(out.status.success());

        let received: Vec<UiEvent> = rx.try_iter().collect();
        assert!(
            received.iter().any(|e| matches!(e, UiEvent::Global(m) if m == "to-pane")),
            "outside direct mode, log() must reach the UI channel"
        );
        assert!(
            received.iter().any(|e| matches!(e, UiEvent::Global(m) if m == "to-cli")),
            "in direct mode, log() must still be recorded in the UI channel (one stream)"
        );
        assert!(
            received.iter().any(|e| matches!(e, UiEvent::UiDevice { detail, .. } if detail.as_deref() == Some("detail"))),
            "in direct mode, log_device() must still be recorded in the UI channel (one stream)"
        );
        assert!(
            received.iter().any(|e| matches!(e, UiEvent::Global(m) if m == "pass-echo")),
            "in direct mode, run_logged() must record the command output in the UI channel"
        );
        assert!(
            received.iter().any(|e| matches!(e, UiEvent::Global(m) if m == "normal-echo")),
            "outside direct mode, run_logged() must log the command output"
        );
    }
}
