//! Configuration loading module
//!
//! This module handles loading and saving of configuration files.

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value};

use super::{Conf, Machine, CONFIG_DIR, DEVICES_DIR, GLOBAL_CONFIG_FILE};

/// Terminate with an error message
fn die(msg: &str) -> ! {
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message
fn warn(msg: &str) {
    eprintln!("WARNING: {}", msg);
}

// ============================================================
// JSON helpers
// ============================================================

/// Parse integer from JSON value
pub fn int_value(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if v.is_number() {
        if let Some(f) = v.as_f64() {
            if f >= 0.0 && f.fract() == 0.0 && f <= u64::MAX as f64 {
                return Some(f as u64);
            }
        }
    }
    None
}

/// Parse number field with validation
pub fn number_field(
    obj: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    match obj.get(key) {
        None => Ok(default),
        Some(v) => {
            let n = match int_value(v) {
                Some(n) => n,
                None => return Err(format!("\"{}\" must be an integer", key)),
            };
            if n < min || n > max {
                return Err(format!("\"{}\" must be between {} and {}", key, min, max));
            }
            Ok(n)
        }
    }
}

// ============================================================
// Device configuration
// ============================================================

/// Load a single device configuration from a JSON file
pub fn load_device_file(path: &Path) -> Result<Machine, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Err(format!("cannot read: {}", e)),
    };
    
    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON: {}", e)),
    };
    
    let obj = match root.as_object() {
        Some(o) => o,
        None => return Err(format!("top-level value must be a JSON object")),
    };
    
    let name = match obj.get("name") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("\"name\" must be a string")),
        },
        None => {
            // Try to get name from filename
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
            if let Some(name) = filename.strip_suffix(".json") {
                name.to_string()
            } else {
                filename.to_string()
            }
        }
    };
    
    let fingerprint = match obj.get("fingerprint") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("\"fingerprint\" must be a string")),
        },
        None => return Err(format!("\"fingerprint\" is required")),
    };
    
    let user = match obj.get("user") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("\"user\" must be a string")),
        },
        None => String::new(),
    };
    
    let (port, port_explicit) = match obj.get("port") {
        None => (22u16, false),
        Some(v) => {
            let n = match int_value(v) {
                Some(n) => n,
                None => return Err(format!("\"port\" must be an integer")),
            };
            if n < 1 || n > 65535 {
                return Err(format!(
                    "\"port\" must be between 1 and 65535"
                ));
            }
            (n as u16, true)
        }
    };
    
    let machine = Machine {
        name,
        fingerprint,
        user,
        port,
        port_explicit,
    };
    
    // Validate the machine configuration
    super::validation::validate_machine(&machine)?;
    
    Ok(machine)
}

/// Save a device configuration to a JSON file
pub fn save_device_file(path: &Path, machine: &Machine) -> io::Result<()> {
    let mut obj = serde_json::Map::new();
    obj.insert("name".to_string(), serde_json::Value::String(machine.name.clone()));
    obj.insert("fingerprint".to_string(), serde_json::Value::String(machine.fingerprint.clone()));
    
    if !machine.user.is_empty() {
        obj.insert("user".to_string(), serde_json::Value::String(machine.user.clone()));
    }
    
    if machine.port_explicit {
        obj.insert("port".to_string(), serde_json::Value::Number(machine.port.into()));
    }
    
    let value = serde_json::Value::Object(obj);
    fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

/// Read device file content as string
pub fn read_device_file_content(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

// ============================================================
// Local identity helpers
// ============================================================

/// Get the local hostname
pub fn host_name() -> String {
    if let Ok(o) = Command::new("hostname").output() {
        if o.status.success() {
            let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !n.is_empty() {
                return n;
            }
        }
    }
    "machine".to_string()
}

/// Get the local IP address
pub fn my_ip() -> Option<String> {
    use std::net::{Ipv4Addr, UdpSocket};
    
    // "Connecting" a UDP socket sends no packet but selects the default
    // interface and reveals its source address.
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("1.1.1.1:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                let ip = a.ip().to_string();
                if ip.parse::<Ipv4Addr>().is_ok() {
                    return Some(ip);
                }
            }
        }
    }
    // Fallback: ask the kernel via "ip route get".
    if let Ok(o) = Command::new("ip")
        .args(["route", "get", "1.1.1.1"])
        .output()
    {
        if o.status.success() {
            let words: Vec<String> = String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(ToString::to_string)
                .collect();
            for (i, w) in words.iter().enumerate() {
                if *w == "src" && i + 1 < words.len() {
                    return Some(words[i + 1].to_string());
                }
            }
        }
    }
    None
}

/// Get the fingerprint of the local machine
pub fn get_local_fingerprint(port: u16) -> Option<String> {
    if let Some(ip) = my_ip() {
        let fps = crate::discovery::fingerprints(&ip, port, false);
        if !fps.is_empty() {
            return Some(fps[0].clone());
        }
    }
    None
}

// ============================================================
// Auto-create device configuration
// ============================================================

/// Auto-create device configuration with self-detected information
pub fn create_device_config(config_dir: &std::path::PathBuf) -> Result<Machine, String> {
    let hostname = host_name();
    
    // Validate hostname
    if hostname.to_lowercase() == "localhost" || hostname.to_lowercase() == "127.0.0.1" {
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
// Global configuration
// ============================================================

/// Default global settings
pub fn default_global_settings() -> (u16, u64, usize, Vec<String>) {
    (4242u16, 10u64, 64usize, Vec::<String>::new())
}

/// Load global settings from settings.json
pub fn load_global_settings(config_dir: &std::path::PathBuf) -> (u16, u64, usize, Vec<String>) {
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    
    let defaults = default_global_settings();
    
    if !settings_path.exists() {
        return defaults;
    }
    
    let text = match fs::read_to_string(&settings_path) {
        Ok(t) => t,
        Err(_) => return defaults,
    };
    
    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return defaults,
    };
    
    let obj = match root.as_object() {
        Some(o) => o,
        None => return defaults,
    };
    
    let udp_port = number_field(obj, "udp_port", 4242, 1, 65535).unwrap_or(4242) as u16;
    let wait_secs = number_field(obj, "wait_secs", 10, 1, 3600).unwrap_or(10);
    let batch_size = number_field(obj, "batch_size", 64, 1, 1024).unwrap_or(64) as usize;
    
    let mut subnets: Vec<String> = Vec::new();
    if let Some(v) = obj.get("subnets") {
        let arr = match v.as_array() {
            Some(a) => a,
            None => return defaults,
        };
        for s in arr {
            let s = match s.as_str() {
                Some(s) => s.trim(),
                None => continue,
            };
            if s.is_empty() {
                continue;
            }
            if let Some(n) = crate::discovery::net_valid(&crate::discovery::net_prefix(s)) {
                subnets.push(n);
            }
        }
    }
    
    (udp_port, wait_secs, batch_size, subnets)
}

/// Save default global settings
pub fn save_default_global_settings(config_dir: &std::path::PathBuf) -> io::Result<()> {
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    let settings = serde_json::json!({
        "udp_port": 4242,
        "wait_secs": 10,
        "batch_size": 64,
        "subnets": []
    });
    
    fs::write(settings_path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
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
    let config_dir = std::path::PathBuf::from(&home).join(CONFIG_DIR);
    
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
    let (udp_port, wait_secs, batch_size, subnets) = load_global_settings(&config_dir);
    
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
        subnets,
        machines,
    }
}
