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

pub mod name;
pub use name::DeviceName;

/// Represents a machine/device in the network
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Machine {
    pub name: DeviceName,
    /// Unique identifier of the machine: its SSH *server* host public key.
    /// Typed to prevent confusion with fingerprints or signing keys.
    pub ssh_public_key: crate::ssh::keys::SshPublicKey,
    /// Ed25519 signing public key (base64) this machine used in its beacon
    /// when it was accepted: future beacons must be signed with this same
    /// key (spoofing detection, reinstall confirmation). Written by regroup
    /// when the device file is created or the key is re-trusted.
    pub signing_public_key: crate::ssh::keys::SigningPublicKeyB64,
    pub user: String,
    pub port: u16,
    pub port_explicit: bool,
}

/// Global discovery settings (udp_port, wait_secs)
#[derive(Clone, Debug)]
pub struct GlobalSettings {
    pub udp_port: u16,
    pub wait_secs: u64,
}

/// Global configuration containing all settings and machines
#[derive(Clone, Debug)]
pub struct Conf {
    pub udp_port: u16,
    pub wait_secs: u64,
    pub machines: Vec<Machine>,
}

// ============================================================
// Configuration constants
// ============================================================

pub const CONFIG_DIR: &str = ".regroup-config";
pub const GIT_DIR: &str = "git";
/// Root of the per-device layout: each device owns one directory
/// `devices/<name>/` and is the only writer inside it (except its device
/// file, see below).
pub const DEVICES_DIR: &str = "devices";
/// Per-device snapshot of the latest known state (inside a device dir).
pub const DEVICE_FILE: &str = "device.json";
/// Known users of a device: one file per user (inside a device dir).
pub const USERS_DIR: &str = "users";
pub const GLOBAL_CONFIG_FILE: &str = "settings.json";

// ============================================================
// Storage layout helpers
// ============================================================

/// Directory of a device's own data: `<config>/devices/<name>/`.
pub fn device_dir(config_dir: &std::path::Path, device: &(impl AsRef<str> + ?Sized)) -> std::path::PathBuf {
    config_dir.join(DEVICES_DIR).join(device.as_ref())
}

/// Path of a device's snapshot file: `<config>/devices/<name>/device.json`.
pub fn device_file_path(config_dir: &std::path::Path, device: &(impl AsRef<str> + ?Sized)) -> std::path::PathBuf {
    device_dir(config_dir, device).join(DEVICE_FILE)
}

// ============================================================
// Submodules
// ============================================================

pub mod connection;
pub mod device;
pub mod json_helpers;
pub mod loading;
pub mod settings;
pub mod users;
pub mod validation;

// Re-export commonly used items
pub use loading::load_config;
