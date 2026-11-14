//! Configuration loading module
//!
//! This module handles the main configuration loading logic.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::device::{load_device_file, save_device_file};
use super::settings::{load_global_settings, save_default_global_settings};
use super::users::ensure_local_user_file;
use super::{
    CONFIG_DIR, Conf, DeviceName, DEVICES_DIR, GLOBAL_CONFIG_FILE, GlobalSettings,
    Device, device_dir, device_file_path,
};
use crate::discovery::local::host_name;
use crate::git::{check_repo, git_init};
use crate::ssh::keys::SigningPublicKeyB64;
use crate::ssh::public_key::get_ssh_server_public_key;

/// Track warnings that have already been displayed to avoid duplicates
static WARNING_TRACKER: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Terminate with an error message
fn die(msg: &str) -> ! {
    // Restore the terminal first: process::exit runs no destructors.
    crate::ui::restore();
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message only once per execution
fn warn(msg: &str) {
    let tracker = WARNING_TRACKER.get_or_init(|| Mutex::new(HashSet::new()));
    let mut tracker = tracker.lock().unwrap();
    if tracker.insert(msg.to_string()) {
        crate::ui::log(&format!("WARNING: {}", msg));
    }
}

// ============================================================
// Auto-create device configuration
// ============================================================

/// Auto-create device configuration with self-detected information
pub fn create_device_config(config_dir: &Path) -> Result<Device, String> {
    let hostname = host_name();

    // Validate hostname
    let hostname_lower = hostname.to_lowercase();
    if hostname_lower == "localhost" || hostname_lower == "127.0.0.1" || hostname_lower == "local" {
        return Err(format!(
            "Hostname '{}' is not valid for device discovery. Please ensure the system has a proper hostname.",
            hostname
        ));
    }

    let ssh_public_key = get_ssh_server_public_key().ok_or_else(|| {
        "Could not detect SSH public key. Ensure SSH server is running on this device.".to_string()
    })?;

    // Pin our own signing key in the device file: the devices that accept
    // this device verify our beacons against it.
    let signing_public_key = match crate::signing::initialize_signing_keys() {
        Ok((_signing_key, verifying_key)) => {
            use base64::Engine;
            SigningPublicKeyB64::from(
                base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes()),
            )
        }
        Err(e) => return Err(format!("cannot initialize signing keys: {}", e)),
    };

    let device = Device {
        name: DeviceName::new(hostname.clone())
            .ok_or_else(|| format!("hostname '{}' is empty or whitespace", hostname))?,
        ssh_public_key,
        signing_public_key,
        port: 22,
        port_explicit: false,
    };

    // Validate the device configuration
    super::validation::validate_device(&device)?;

    // Save to the device's own directory
    let dir = device_dir(config_dir, &hostname);
    if let Err(e) = fs::create_dir_all(&dir) {
        return Err(format!("cannot create device directory: {}", e));
    }

    let device_path = device_file_path(config_dir, &hostname);
    if let Err(e) = save_device_file(&device_path, &device) {
        return Err(format!("cannot save device config: {}", e));
    }

    // First run: the device file is the repository's initial commit, so the
    // local branch exists before any sync with another device (a removal
    // on a commit-less repository cannot be committed).
    if let Err(e) = crate::git::sync::git_add_and_commit_file(
        config_dir,
        &device_path,
        &format!("Add device {hostname}"),
    ) {
        warn(&format!("could not commit device config: {}", e));
    }

    Ok(device)
}

// ============================================================
// Main configuration loading
// ============================================================

/// Load the complete configuration
///
/// Configuration structure:
/// ~/.regroup-config/              - Root config directory (NOT a git repo)
///   ├── settings.json             - Local-only settings (udp_port, wait_secs),
///   │                               never synced
///   ├── git/                      - Git repository: source of truth, synced
///   │   └── devices/<name>/       - One directory per device, written
///   │       ├── device.json       -   only by that device (see below)
///   │       ├── users/            -   known users of the device
///   │       └── connections/      -   connections to/from the device
///   └── tmp/                      - Temporary overlay (mode 0700), never
///                                   committed or synced: placeholder
///                                   device.json files and temporary users
///
/// All reads go through the configuration proxy: the effective view is
/// `git/ ∪ (tmp/ − git/)`, `git/` wins on overlap and stale `tmp/` copies
/// are purged (lazily on read, eagerly after each sync). Each device only
/// ever writes inside its own directory in `git/`; other devices discover
/// its files through git sync.
pub fn load_config() -> Conf {
    // Root config directory
    let home = match env::var("HOME") {
        Ok(h) if !h.is_empty() => h,
        _ => die("HOME variable not set"),
    };
    let root_config_dir = PathBuf::from(&home).join(CONFIG_DIR);

    // Git config directory (this IS the git repo)
    let config_dir = root_config_dir.join(super::GIT_DIR);

    // Ensure root config directory exists
    if let Err(e) = fs::create_dir_all(&root_config_dir) {
        die(&format!(
            "cannot initialize config directory {}: {}",
            root_config_dir.display(),
            e
        ));
    }

    // Ensure git config directory exists and is a git repository
    if let Err(e) = fs::create_dir_all(&config_dir) {
        die(&format!(
            "cannot initialize git config directory {}: {}",
            config_dir.display(),
            e
        ));
    }

    // Initialize git repository in the git/ subdirectory
    if let Err(e) = git_init(&config_dir) {
        die(&format!(
            "cannot initialize git repository in {}: {}",
            config_dir.display(),
            e
        ));
    }

    // Create the tmp/ overlay (mode 0700): temporary, never-synced data
    if let Err(e) = super::proxy::ensure_tmp_dir(&root_config_dir) {
        die(&format!(
            "cannot initialize tmp overlay in {}: {}",
            root_config_dir.join(super::TMP_DIR).display(),
            e
        ));
    }

    // Repository checks: exists, required settings configured, clean tree.
    // A failing check must not prevent reading the configuration, so it
    // only warns here.
    if let Err(e) = check_repo(&config_dir) {
        warn(&format!("git repository check: {}", e));
    }

    // Create default settings (outside the repo) if they don't exist
    if !root_config_dir.join(GLOBAL_CONFIG_FILE).exists()
        && let Err(e) = save_default_global_settings(&root_config_dir)
    {
        die(&format!("cannot create default settings: {}", e));
    }

    // Create devices directory in the git layer
    let devices_dir = config_dir.join(DEVICES_DIR);
    if let Err(e) = fs::create_dir_all(&devices_dir) {
        die(&format!("cannot create devices directory: {}", e));
    }

    // Load global settings (local-only, outside the repo)
    let GlobalSettings {
        udp_port,
        wait_secs,
    } = load_global_settings(&root_config_dir);

    // Load all device configurations through the proxy: one effective
    // device.json per device, merging git/devices/ with tmp/devices/
    // (git/ wins on overlap, overlapping tmp/ files purged lazily).
    let mut devices: Vec<Device> = Vec::new();

    for name in super::proxy::merged_device_names(&root_config_dir) {
        if let Some(device_path) = super::proxy::device_file(&root_config_dir, &name) {
            match load_device_file(&device_path) {
                Ok(device) => devices.push(device),
                Err(e) => warn(&format!("skipping {}: {}", device_path.display(), e)),
            }
        }
    }

    // Check if current device's config exists, if not create it
    let local_hostname = host_name();
    let local_ssh_public_key = get_ssh_server_public_key();

    let current_device_exists = devices.iter().any(|d| {
        d.name.as_str() == local_hostname || (local_ssh_public_key.as_ref() == Some(&d.ssh_public_key))
    });

    if !current_device_exists {
        // Auto-create this device's configuration
        match create_device_config(&config_dir) {
            Ok(new_device) => {
                devices.push(new_device);
            }
            Err(e) => {
                warn(&format!("could not create device config: {}", e));
            }
        }
    }

    // Record the current user on the local device (first run only):
    // `devices/<self>/users/<user>.json` is written once and committed,
    // so other devices learn who this device's users are through git.
    if devices.iter().any(|d| d.name.as_str() == local_hostname) {
        let current_user =
            env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
        if !current_user.is_empty() {
            match ensure_local_user_file(&config_dir, &local_hostname, &current_user) {
                Ok(true) => {
                    let user_path = super::users::user_file_path(
                        &config_dir,
                        &local_hostname,
                        &current_user,
                    );
                    if let Err(e) = crate::git::sync::git_add_and_commit_file(
                        &config_dir,
                        &user_path,
                        &format!("Add user {current_user} on {local_hostname}"),
                    ) {
                        warn(&format!("could not commit user file: {}", e));
                    }
                }
                Ok(false) => {}
                Err(e) => warn(&format!(
                    "could not record user {current_user} on {local_hostname}: {}",
                    e
                )),
            }
        }
    }

    // Sort devices by name for consistent ordering
    devices.sort_by(|a, b| a.name.as_str().cmp(&b.name.as_str()));

    if devices.is_empty() {
        warn(&format!(
            "no devices configured in {} — first device will be auto-created on first run",
            config_dir.display()
        ));
    }

    Conf {
        udp_port,
        wait_secs,
        devices,
    }
}


