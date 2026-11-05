//! TUI actions for installing and removing SSH keys
#![allow(dead_code)]
//!
//! This module provides simple TUI-based functions for:
//! - Installing SSH keys on remote machines
//! - Removing SSH keys from remote machines
//!
//! These are implemented as standalone functions that can be called from
//! the connections screen.

use std::path::Path;
use std::process::Command;

use chrono;

use crate::cli::key_management::{generate_key_pair, get_public_key, is_reachable, build_connection_id, get_config_dir};
use crate::config::connection::{Connection, save_connection};
use crate::git::sync::git_add_and_commit_file;

/// Result of a key action
#[derive(Debug)]
pub enum KeyActionResult {
    /// Action completed successfully
    Success(String),
    /// Action was cancelled by user
    Cancelled,
    /// Error occurred
    Error(String),
}

/// Install a key on a remote machine
pub fn install_key_action(
    _home: &str,
    config_dir: &Path,
    local_machine: &str,
    current_user: &str,
    to_machine: &str,
    to_user: &str,
) -> Result<KeyActionResult, String> {
    let connection_id = build_connection_id(current_user, to_user, to_machine);
    
    // Check if a connection already exists for this pair
    let connections_dir = config_dir.join("devices").join(local_machine).join("connections");
    let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_machine);
    let connection_path = connections_dir.join(connection_file_name);
    
    if connection_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "Connection already exists for {}. Remove it first if you want to recreate it.",
            connection_id
        )));
    }
    
    // Check if machine is reachable
    if !is_reachable(to_machine, 22, to_user) {
        return Ok(KeyActionResult::Error(format!("Machine {} unreachable", to_machine)));
    }
    
    // Check if key already exists
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let key_path = std::path::PathBuf::from(&home).join(".ssh").join(format!("id_regroup_{}@{}", current_user, to_machine));
    
    if key_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "Key already exists for this connection ({}). Regroup manages only one key per connection.",
            key_path.display()
        )));
    }
    
    // Generate key pair
    let key_path = generate_key_pair(current_user, to_machine)
        .map_err(|e| format!("Failed to generate key: {}", e))?;
    
    let public_key = get_public_key(&key_path)
        .map_err(|e| format!("Failed to read public key: {}", e))?;
    
    // Build the command for the remote machine
    let cmd = format!(
        "regroup receive-install-key --source-user {} --source-machine {} --public-key '{}' --connection-id {}",
        current_user, local_machine, public_key, connection_id
    );
    
    // Execute SSH command to install key on remote
    let ssh_cmd = format!("ssh {}@{} '{}'", to_user, to_machine, cmd);
    
    // Temporarily disable raw mode for SSH
    crossterm::terminal::disable_raw_mode().map_err(|e| format!("Cannot disable raw mode: {}", e))?;
    
    let status = Command::new("sh").arg("-c").arg(&ssh_cmd).status();
    
    // Re-enable raw mode
    let _ = crossterm::terminal::enable_raw_mode();
    
    match status {
        Ok(exit_status) if exit_status.success() => {
            // Save connection file on source machine
            let connection = Connection::new(
                current_user,
                local_machine,
                to_user,
                to_machine,
                &public_key,
                &key_path.to_string_lossy(),
                &chrono::Utc::now().to_rfc3339(),
            );
            
            save_connection(
                config_dir,
                local_machine,
                &connection,
                current_user,
            ).map_err(|e| format!("Failed to save connection: {}", e))?;
            
            // Commit on source machine
            let connections_dir = config_dir.join("devices").join(local_machine).join("connections");
            let config_dir_path = get_config_dir();
            
            git_add_and_commit_file(
                &config_dir_path,
                &connections_dir,
                &format!("Add connection: {}->{}@{}", current_user, to_user, to_machine),
            ).map_err(|e| format!("Failed to commit: {}", e))?;
            
            Ok(KeyActionResult::Success(format!("Key installed successfully on {}", to_machine)))
        }
        Ok(_) => Ok(KeyActionResult::Error(format!("Failed to install on {}", to_machine))),
        Err(e) => Ok(KeyActionResult::Error(format!("SSH error: {}", e))),
    }
}

/// Remove a key from a remote machine
pub fn remove_key_action(
    _home: &str,
    config_dir: &Path,
    local_machine: &str,
    current_user: &str,
    _from_user: &str,
    to_machine: &str,
    to_user: &str,
    connection_id: &str,
) -> Result<KeyActionResult, String> {
    // Check if machine is reachable
    if !is_reachable(to_machine, 22, to_user) {
        return Ok(KeyActionResult::Error(format!("Machine {} unreachable", to_machine)));
    }
    
    // Check if connection file exists locally
    let connections_dir = config_dir.join("devices").join(local_machine).join("connections");
    let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_machine);
    let connection_path = connections_dir.join(connection_file_name);
    
    if !connection_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "No connection found for {}->{}@{} on this machine",
            current_user, to_user, to_machine
        )));
    }
    
    // Build the command for the remote machine
    let cmd = format!(
        "regroup receive-remove-key --connection-id {}",
        connection_id
    );
    
    // Execute SSH command to remove key on remote
    let ssh_cmd = format!("ssh {}@{} '{}'", to_user, to_machine, cmd);
    
    // Temporarily disable raw mode for SSH
    crossterm::terminal::disable_raw_mode().map_err(|e| format!("Cannot disable raw mode: {}", e))?;
    
    let status = Command::new("sh").arg("-c").arg(&ssh_cmd).status();
    
    // Re-enable raw mode
    let _ = crossterm::terminal::enable_raw_mode();
    
    match status {
        Ok(exit_status) if exit_status.success() => {
            // Delete connection file on source machine
            let connections_dir = config_dir.join("devices").join(local_machine).join("connections");
            let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_machine);
            let connection_path = connections_dir.join(connection_file_name);
            
            if connection_path.exists() {
                std::fs::remove_file(&connection_path)
                    .map_err(|e| format!("Failed to remove connection file: {}", e))?;
            }
            
            // Commit on source machine
            let config_dir_path = get_config_dir();
            
            git_add_and_commit_file(
                &config_dir_path,
                &connections_dir,
                &format!("Remove connection: {}->{}@{}", current_user, to_user, to_machine),
            ).map_err(|e| format!("Failed to commit: {}", e))?;
            
            Ok(KeyActionResult::Success(format!("Key removed successfully from {}", to_machine)))
        }
        Ok(_) => Ok(KeyActionResult::Error(format!("Failed to remove from {}", to_machine))),
        Err(e) => Ok(KeyActionResult::Error(format!("SSH error: {}", e))),
    }
}
