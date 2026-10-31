//! SSH configuration helpers module
//!
//! This module provides utilities for working with SSH configuration files.

use std::fs;
use std::path::Path;

/// Verify that SSH config contains entries for all machines
/// Returns list of machine names missing from SSH config
pub fn verify_ssh_config_complete(ssh_config: &Path, machines: &[crate::config::Machine]) -> Vec<String> {
    let ssh_content = match fs::read_to_string(ssh_config) {
        Ok(c) => c,
        Err(_) => return machines.iter().map(|m| m.name.clone()).collect(),
    };
    
    let mut missing = Vec::new();
    for m in machines {
        // Skip local machine - it shouldn't be in SSH config pointing to itself
        if !ssh_content.contains(&format!("Host {}", m.name)) {
            missing.push(m.name.clone());
        }
    }
    missing
}

/// Force SSH config file to be written to disk
pub fn flush_ssh_config(ssh_config: &Path) -> std::io::Result<()> {
    // Open in append mode to ensure file exists, then sync
    let file = fs::OpenOptions::new()
        .append(true)
        .open(ssh_config)?;
    file.sync_all()?;
    Ok(())
}
