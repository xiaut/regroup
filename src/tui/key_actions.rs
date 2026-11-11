//! TUI actions for installing and removing SSH keys
#![allow(dead_code)]
//!
//! This module provides simple TUI-based functions for:
//! - Installing SSH keys on remote devices
//! - Removing SSH keys from remote devices
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

/// Install a key on a remote device
pub fn install_key_action(
    _home: &str,
    config_dir: &Path,
    local_device: &str,
    current_user: &str,
    to_device: &str,
    to_user: &str,
    to_port: u16,
) -> Result<KeyActionResult, String> {
    let connection_id = build_connection_id(current_user, to_user, to_device);
    
    // Check if a connection already exists for this pair
    let connections_dir = config_dir.join("devices").join(local_device).join("connections");
    let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_device);
    let connection_path = connections_dir.join(connection_file_name);
    
    if connection_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "Connection already exists for {}. Remove it first if you want to recreate it.",
            connection_id
        )));
    }
    
    // Check if device is reachable
    if !is_reachable(to_device, to_port, to_user) {
        return Ok(KeyActionResult::Error(format!("Device {} unreachable", to_device)));
    }
    
    // Check if key already exists
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let key_path = std::path::PathBuf::from(&home).join(".ssh").join(format!("id_regroup_{}@{}", current_user, to_device));
    
    if key_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "Key already exists for this connection ({}). Regroup manages only one key per connection.",
            key_path.display()
        )));
    }
    
    // Generate key pair
    let key_path = generate_key_pair(current_user, to_device)
        .map_err(|e| format!("Failed to generate key: {}", e))?;
    
    let public_key = get_public_key(&key_path)
        .map_err(|e| format!("Failed to read public key: {}", e))?;
    
    // Log SSH connection details
    crate::ui::log(&format!(
        "Connecting via SSH to {}@{} on port {} to install key...",
        to_user, to_device, to_port
    ));
    crate::ui::log(
        "Note: If the remote device requires password authentication, you will be prompted for a password."
    );
    
    // Build the command for the remote device
    // Use separate arguments to avoid shell quoting issues
    let mut ssh_cmd = Command::new("ssh");
    ssh_cmd.arg(format!("{}@{}", to_user, to_device));
    if to_port != 22 {
        ssh_cmd.arg("-p").arg(to_port.to_string());
    }
    let output = ssh_cmd
        .arg("regroup")
        .arg("receive-install-key")
        .arg("--source-user")
        .arg(current_user)
        .arg("--source-device")
        .arg(local_device)
        .arg("--public-key")
        .arg(&public_key)
        .arg("--connection-id")
        .arg(connection_id)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    
    match output {
        Ok(output) => {
            // Log stderr to help user understand what's happening (e.g., password prompts)
            if !output.stderr.is_empty() {
                let stderr_str = String::from_utf8_lossy(&output.stderr);
                for line in stderr_str.lines() {
                    if !line.is_empty() {
                        crate::ui::log(&format!("ssh {}@{}: {}", to_user, to_device, line));
                    }
                }
            }
            
            if output.status.success() {
                // Save connection file on source device
                let connection = Connection::new(
                    current_user,
                    local_device,
                    to_user,
                    to_device,
                    &public_key,
                    &key_path.to_string_lossy(),
                    &chrono::Utc::now().to_rfc3339(),
                );
                
                save_connection(
                    config_dir,
                    local_device,
                    &connection,
                    current_user,
                ).map_err(|e| format!("Failed to save connection: {}", e))?;
                
                // Commit on source device
                let connections_dir = config_dir.join("devices").join(local_device).join("connections");
                let config_dir_path = get_config_dir();
                
                git_add_and_commit_file(
                    &config_dir_path,
                    &connections_dir,
                    &format!("Add connection: {}->{}@{}", current_user, to_user, to_device),
                ).map_err(|e| format!("Failed to commit: {}", e))?;
                
                Ok(KeyActionResult::Success(format!("Key installed successfully on {}", to_device)))
            } else {
                Ok(KeyActionResult::Error(format!("Failed to install on {}", to_device)))
            }
        }
        Err(e) => Ok(KeyActionResult::Error(format!("SSH error: {}", e))),
    }
}

/// Remove a key from a remote device
pub fn remove_key_action(
    _home: &str,
    config_dir: &Path,
    local_device: &str,
    current_user: &str,
    _from_user: &str,
    to_device: &str,
    to_user: &str,
    to_port: u16,
    connection_id: &str,
) -> Result<KeyActionResult, String> {
    // Check if device is reachable
    if !is_reachable(to_device, to_port, to_user) {
        return Ok(KeyActionResult::Error(format!("Device {} unreachable", to_device)));
    }
    
    // Check if connection file exists locally
    let connections_dir = config_dir.join("devices").join(local_device).join("connections");
    let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_device);
    let connection_path = connections_dir.join(connection_file_name);
    
    if !connection_path.exists() {
        return Ok(KeyActionResult::Error(format!(
            "No connection found for {}->{}@{} on this device",
            current_user, to_user, to_device
        )));
    }
    
    // Log SSH connection details
    crate::ui::log(&format!(
        "Connecting via SSH to {}@{} on port {} to remove key...",
        to_user, to_device, to_port
    ));
    crate::ui::log(
        "Note: If the remote device requires password authentication, you will be prompted for a password."
    );
    
    // Build the command for the remote device
    // Use separate arguments to avoid shell quoting issues
    let mut ssh_cmd = Command::new("ssh");
    ssh_cmd.arg(format!("{}@{}", to_user, to_device));
    if to_port != 22 {
        ssh_cmd.arg("-p").arg(to_port.to_string());
    }
    let output = ssh_cmd
        .arg("regroup")
        .arg("receive-remove-key")
        .arg("--connection-id")
        .arg(connection_id)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    
    match output {
        Ok(output) => {
            // Log stderr to help user understand what's happening (e.g., password prompts)
            if !output.stderr.is_empty() {
                let stderr_str = String::from_utf8_lossy(&output.stderr);
                for line in stderr_str.lines() {
                    if !line.is_empty() {
                        crate::ui::log(&format!("ssh {}@{}: {}", to_user, to_device, line));
                    }
                }
            }
            
            if output.status.success() {
                // Delete connection file on source device
                let connections_dir = config_dir.join("devices").join(local_device).join("connections");
                let connection_file_name = format!("{}->{}@{}.json", current_user, to_user, to_device);
                let connection_path = connections_dir.join(connection_file_name);
                
                if connection_path.exists() {
                    std::fs::remove_file(&connection_path)
                        .map_err(|e| format!("Failed to remove connection file: {}", e))?;
                }
                
                // Commit on source device
                let config_dir_path = get_config_dir();
                
                git_add_and_commit_file(
                    &config_dir_path,
                    &connections_dir,
                    &format!("Remove connection: {}->{}@{}", current_user, to_user, to_device),
                ).map_err(|e| format!("Failed to commit: {}", e))?;
                
                Ok(KeyActionResult::Success(format!("Key removed successfully from {}", to_device)))
            } else {
                Ok(KeyActionResult::Error(format!("Failed to remove from {}", to_device)))
            }
        }
        Err(e) => Ok(KeyActionResult::Error(format!("SSH error: {}", e))),
    }
}
