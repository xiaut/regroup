//! Network types module
//!
//! This module holds the local network interface type used for beacon
//! announcement and discovery.

/// Represents a local network interface with its IP address and broadcast address.
#[derive(Clone, Debug)]
pub struct NetworkInterface {
    pub ip: String,
    pub broadcast: String,
}
