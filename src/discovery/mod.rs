//! Discovery module for regroup
//!
//! This module handles all network discovery functionality including:
//! - UDP beacon announcement and listening
//! - Local identity detection

pub mod beacon;
pub mod local;
pub mod net;

// Re-export commonly used types
pub use net::NetworkInterface;

// Re-export commonly used functions
pub use local::{get_local_interfaces, host_name, my_ip};
