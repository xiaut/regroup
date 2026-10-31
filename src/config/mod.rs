//! Configuration module for regroup
//!
//! This module handles all configuration-related functionality.

// ============================================================
// Data types
// ============================================================

/// Represents a machine/device in the network
#[derive(Clone, Debug)]
pub struct Machine {
    pub name: String,
    pub fingerprint: String,
    pub user: String,
    pub port: u16,
    pub port_explicit: bool,
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
