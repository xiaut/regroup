//! Configuration loading module
//!
//! This module handles the main configuration loading logic.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::{Conf, GlobalSettings, Machine, CONFIG_DIR, DEVICES_DIR, GLOBAL_CONFIG_FILE};
use super::device::{load_device_file, save_device_file};
use super::settings::{load_global_settings, save_default_global_settings};
use crate::discovery::local::host_name;
use crate::ssh::keys::SigningPublicKeyB64;
use crate::ssh::public_key::get_ssh_server_public_key;
use crate::git::git_init;

/// Track warnings that have already been displayed to avoid duplicates
static WARNING_TRACKER: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Terminate with an error message
fn die(msg: &str) -> ! {
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message only once per execution
fn warn(msg: &str) {
    let tracker = WARNING_TRACKER.get_or_init(|| Mutex::new(HashSet::new()));
    let mut tracker = tracker.lock().unwrap();
    if tracker.insert(msg.to_string()) {
        eprintln!("WARNING: {}", msg);
    }
}

// ============================================================
// Auto-create device configuration
// ============================================================

/// Auto-create device configuration with self-detected information
pub fn create_device_config(config_dir: &PathBuf) -> Result<Machine, String> {
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
        "Could not detect SSH public key. Ensure SSH server is running on this machine.".to_string()
    })?;

    // Pin our own signing key in the device file: the machines that accept
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
    
    // Determine the current user
    let current_user = env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
    
    let machine = Machine {
        name: hostname.clone(),
        ssh_public_key,
        signing_public_key,
        user: current_user,
        port: 22,
        port_explicit: false,
    };
    
    // Validate the machine configuration
    super::validation::validate_machine(&machine)?;
    
    // Save to device file
    let devices_dir = config_dir.join(DEVICES_DIR);
    if !devices_dir.exists() {
        if let Err(e) = fs::create_dir_all(&devices_dir) {
            return Err(format!("cannot create devices directory: {}", e));
        }
    }
    
    let device_path = devices_dir.join(format!("{}.json", hostname));
    if let Err(e) = save_device_file(&device_path, &machine) {
        return Err(format!("cannot save device config: {}", e));
    }
    
    Ok(machine)
}

// ============================================================
// Main configuration loading
// ============================================================

/// Load the complete configuration
/// 
/// Configuration structure:
/// ~/.regroup-config/              - Root config directory (NOT a git repo)
///   └── git/                      - Git repository with shared config
///       ├── devices/              - Device configs
///       └── settings.json         - Global settings
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
        die(&format!("cannot initialize config directory {}: {}", root_config_dir.display(), e));
    }
    
    // Ensure git config directory exists and is a git repository
    if let Err(e) = fs::create_dir_all(&config_dir) {
        die(&format!("cannot initialize git config directory {}: {}", config_dir.display(), e));
    }
    
    // Initialize git repository in the git/ subdirectory
    if let Err(e) = git_init(&config_dir) {
        die(&format!("cannot initialize git repository in {}: {}", config_dir.display(), e));
    }
    
    // Create default settings if they don't exist
    if !config_dir.join(GLOBAL_CONFIG_FILE).exists() {
        if let Err(e) = save_default_global_settings(&config_dir) {
            die(&format!("cannot create default settings: {}", e));
        }
    }
    
    // Create devices directory
    let devices_dir = config_dir.join(DEVICES_DIR);
    if let Err(e) = fs::create_dir_all(&devices_dir) {
        die(&format!("cannot create devices directory: {}", e));
    }
    
    // Load global settings
    let GlobalSettings { udp_port, wait_secs } = load_global_settings(&config_dir);
    
    // Load all device configurations
    let mut machines: Vec<Machine> = Vec::new();
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    if devices_dir.exists() {
        if let Ok(entries) = fs::read_dir(&devices_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                        match load_device_file(&path) {
                            Ok(machine) => machines.push(machine),
                            Err(e) => warn(&format!("skipping {}: {}", path.display(), e)),
                        }
                    }
                }
            }
        }
    }
    
    // Check if current machine's config exists, if not create it
    let local_hostname = host_name();
    let local_ssh_public_key = get_ssh_server_public_key();
    
    let current_machine_exists = machines.iter().any(|m| {
        m.name == local_hostname || 
        (local_ssh_public_key.as_ref().map_or(false, |k| &m.ssh_public_key == k))
    });
    
    if !current_machine_exists {
        // Auto-create this device's configuration
        match create_device_config(&config_dir) {
            Ok(new_machine) => {
                machines.push(new_machine);
            }
            Err(e) => {
                warn(&format!("could not create device config: {}", e));
            }
        }
    }
    
    // Sort machines by name for consistent ordering
    machines.sort_by(|a, b| a.name.cmp(&b.name));
    
    if machines.is_empty() {
        warn(&format!(
            "no devices configured in {} — first device will be auto-created on first run",
            config_dir.display()
        ));
    }
    
    Conf {
        udp_port,
        wait_secs,
        machines,
    }
}
