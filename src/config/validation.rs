//! Configuration validation module
//!
//! This module handles validation of machine configurations.

use super::Machine;

/// Validate a machine configuration
pub fn validate_machine(machine: &Machine) -> Result<(), String> {
    // Check for placeholder fingerprint
    if machine.fingerprint == "SHA256:PUT-YOUR-FINGERPRINT-HERE" ||
       machine.fingerprint == "MD5:PUT-YOUR-FINGERPRINT-HERE" {
        return Err(format!(
            "Device '{}' has a placeholder fingerprint. Please run: ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub to get the real fingerprint",
            machine.name
        ));
    }
    
    // Validate hostname - reject localhost and other invalid names
    let name_lower = machine.name.to_lowercase();
    if name_lower == "localhost" || name_lower == "localhost.localdomain" || name_lower == "127.0.0.1" {
        return Err(format!(
            "Device name '{}' is invalid. Hostname cannot be 'localhost' or '127.0.0.1'. Please use the actual hostname.",
            machine.name
        ));
    }
    
    Ok(())
}
