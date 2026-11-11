//! Configuration validation module
//!
//! This module handles validation of device configurations.

use super::Device;

/// Validate a device configuration
pub fn validate_device(device: &Device) -> Result<(), String> {
    // Check for placeholder SSH public key
    if device.ssh_public_key.as_str() == "SSH-PUBLIC-KEY-PLACEHADER"
        || device.ssh_public_key.is_empty()
    {
        return Err(format!(
            "Device '{}' has a placeholder or empty SSH public key. Please ensure the SSH server is running and the key can be detected.",
            device.name.as_str()
        ));
    }

    // Validate hostname - reject localhost and other invalid names
    let name_lower = device.name.as_str().to_lowercase();
    if name_lower == "localhost"
        || name_lower == "localhost.localdomain"
        || name_lower == "127.0.0.1"
    {
        return Err(format!(
            "Device name '{}' is invalid. Hostname cannot be 'localhost' or '127.0.0.1'. Please use the actual hostname.",
            device.name.as_str()
        ));
    }

    Ok(())
}
