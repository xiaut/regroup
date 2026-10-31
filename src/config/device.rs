//! Device configuration module
//!
//! This module handles loading and saving individual device configurations.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use super::Machine;
use super::json_helpers::int_value;
use super::validation::validate_machine;

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
    validate_machine(&machine)?;
    
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
