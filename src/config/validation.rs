//! Configuration validation module
//!
//! This module handles validation of machine configurations.

use super::Machine;

/// Validate a machine configuration
pub fn validate_machine(machine: &Machine) -> Result<(), String> {
    // Check for placeholder SSH public key
    if machine.ssh_public_key == "SSH-PUBLIC-KEY-PLACEHOLDER" ||
       machine.ssh_public_key.is_empty() {
        return Err(format!(
            "Device '{}' has a placeholder or empty SSH public key. Please ensure the SSH server is running and the key can be detected.",
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
