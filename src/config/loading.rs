//! Configuration loading module
//!
//! This module handles the main configuration loading logic.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::{Conf, Machine, CONFIG_DIR, DEVICES_DIR};
use super::device::{load_device_file, save_device_file};
use super::settings::{load_global_settings, save_default_global_settings};
use crate::discovery::local::{host_name, get_local_fingerprint};

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
    
    let fingerprint = get_local_fingerprint(22).ok_or_else(|| {
        "Could not detect SSH fingerprint. Ensure SSH server is running on this machine.".to_string()
    })?;
    
    // Determine the current user
    let current_user = env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
    
    let machine = Machine {
        name: hostname.clone(),
        fingerprint,
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
pub fn load_config() -> Conf {
    // Config directory is always ~/.regroup-config
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
    
    // Initialize config directory if it doesn't exist
    if !config_dir.exists() {
        if let Err(e) = fs::create_dir_all(&config_dir) {
            die(&format!("cannot initialize config directory {}: {}", config_dir.display(), e));
        }
        
        // Create default settings
        if let Err(e) = save_default_global_settings(&config_dir) {
            die(&format!("cannot create default settings: {}", e));
        }
        
        // Create devices directory
        let devices_dir = config_dir.join(DEVICES_DIR);
        if let Err(e) = fs::create_dir_all(&devices_dir) {
            die(&format!("cannot create devices directory: {}", e));
        }
    }
    
    // Load global settings
    let (udp_port, wait_secs, batch_size) = load_global_settings(&config_dir);
    
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
    let local_fingerprint = get_local_fingerprint(22);
    
    let current_machine_exists = machines.iter().any(|m| {
        m.name == local_hostname || 
        (m.fingerprint.starts_with("SHA256:") && 
         local_fingerprint.as_ref().map_or(false, |f| &m.fingerprint == f))
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
        die(&format!(
            "no devices configured in {} — first device will be auto-created on first run",
            config_dir.display()
        ));
    }
    
    Conf {
        udp_port,
        wait_secs,
        batch_size,
        machines,
    }
}
