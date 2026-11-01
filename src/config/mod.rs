//! Configuration module for regroup
//!
//! This module handles all configuration-related functionality.
//!
//! IMPORTANT: Configuration files are a backup/persistence mechanism, NOT the primary
//! source of truth. The actual source of truth is live device discovery via UDP beacons.
//! Each device self-identifies with its public key, and devices are automatically added
//! to configuration when their beacon (containing name, public_key, ip) is received.
//! The configuration persists what was previously discovered so devices can be
//! reconnected even after restarts.

// ============================================================
// Data types
// ============================================================

/// Represents a machine/device in the network
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Machine {
    pub name: String,
    /// Unique identifier of the machine: its SSH *server* host public key.
    /// Typed to prevent confusion with fingerprints or signing keys.
    pub ssh_public_key: crate::ssh::keys::SshPublicKey,
    pub user: String,
    pub port: u16,
    pub port_explicit: bool,
}

/// Global discovery settings (udp_port, wait_secs, batch_size)
#[derive(Clone, Debug)]
pub struct GlobalSettings {
    pub udp_port: u16,
    pub wait_secs: u64,
    pub batch_size: usize,
}

/// Global configuration containing all settings and machines
#[derive(Clone, Debug)]
pub struct Conf {
    pub udp_port: u16,
    pub wait_secs: u64,
    pub batch_size: usize,
    pub machines: Vec<Machine>,
}

// ============================================================
// Configuration constants
// ============================================================

pub const CONFIG_DIR: &str = ".regroup-config";
pub const GIT_DIR: &str = "git";
pub const DEVICES_DIR: &str = "devices";
pub const GLOBAL_CONFIG_FILE: &str = "settings.json";
pub const MAX_SYNC_ITERATIONS: usize = 10;

// ============================================================
// Submodules
// ============================================================

pub mod device;
pub mod json_helpers;
pub mod loading;
pub mod settings;
pub mod validation;

// Re-export commonly used items
pub use loading::load_config;
pub use device::read_device_file_content;
