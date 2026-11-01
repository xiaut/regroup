//! Discovery module for regroup
//!
//! This module handles all network discovery functionality including:
//! - UDP beacon announcement and listening
//! - TCP subnet scanning
//! - SSH fingerprint detection
//! - Local identity detection
//! - Discovery and sync loop

pub mod beacon;
pub mod fingerprint;
pub mod local;
pub mod net;
pub mod scan;
pub mod sync;

// Re-export commonly used types
pub use beacon::BeaconDevice;
pub use net::NetworkInterface;

/// Result of a discovery sync operation
#[derive(Clone, Debug)]
pub struct DiscoveryResult {
    pub found: usize,
    pub missing: usize,
}

// Re-export commonly used functions
pub use beacon::{announce_beacon, listen_beacons};
pub use local::{get_local_interfaces, host_name, my_ip};
pub use net::{net_prefix, net_valid};
