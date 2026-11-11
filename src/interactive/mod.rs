//! Interactive mode: regroup's main loop.
//!
//! Listens for verified beacons (forwarded by the listener thread through a
//! channel) and for user input (read by the ui module), and manages the
//! pending-device queue, the signing-key rotation prompt and the rename
//! prompt. All decisions that affect what is written to disk live here.
//!
//! General messages go through `ui::log` (activity pane of the full-screen
//! interface, stdout in plain mode); messages about one device go through
//! `ui::log_device`, which the full-screen ui shows as an animated status on
//! the device's row. The current question stays visible as the bottom block
//! at all times.

mod state;
mod utils;

pub use state::{Context, run};
