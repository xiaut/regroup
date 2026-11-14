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
///
/// `to_device` is the logical device name (used in the connection ID and
/// file names); `to_ip` is the address the device advertised in its beacon,
/// used as the actual SSH transport target — the name is never resolved
/// (no DNS, no ~/.ssh/config alias, which regroup writes itself).
pub fn install_key_action(
    _home: &str,
    config_dir: &Path,
    local_device: &str,
    current_user: &str,
    to_device: &str,
    to_ip: &str,
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
    
    // Check if device is reachable at its advertised IP
    if !is_reachable(to_ip, to_port) {
        return Ok(KeyActionResult::Error(format!(
            "Device {} unreachable at {}:{}",
            to_device, to_ip, to_port
        )));
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
        "Connecting via SSH to {}@{} ({}) on port {} to install key...",
        to_user, to_ip, to_device, to_port
    ));
    crate::ui::log(
        "Note: If the remote device requires password authentication, you will be prompted for a password."
    );
    
    // Build the command for the remote device.
    // Options must come BEFORE the destination: everything after `user@host`
    // is sent to the remote as the command.
    let mut ssh_cmd = Command::new("ssh");
    if to_port != 22 {
        ssh_cmd.arg("-p").arg(to_port.to_string());
    }
    // Dial the beacon-advertised IP directly, not the device-name alias
    // (the alias lives in ~/.ssh/config, which regroup writes itself).
    ssh_cmd.arg(format!("{}@{}", to_user, to_ip));
    ssh_cmd
        .arg("regroup")
        .arg("receive-install-key")
        .arg("--source-user")
        .arg(current_user)
        .arg("--source-device")
        .arg(local_device)
        .arg("--public-key")
        .arg(&public_key)
        .arg("--connection-id")
        .arg(connection_id);
    // Inherit the terminal so ssh can prompt for a password interactively: in
    // the TUI's temporary CLI mode this process's stdio IS the user's
    // terminal. Capturing the streams (Command::output) nulls the child's
    // stdin and only shows stderr after exit, which makes password auth
    // impossible — ssh fails with "Permission denied (publickey,password)"
    // without ever prompting.
    let status = ssh_cmd
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    
    match status {
        Ok(status) => {
            if status.success() {
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
///
/// `to_device` is the logical device name; `to_ip` is the beacon-advertised
/// address used as the actual SSH transport target (see install_key_action).
pub fn remove_key_action(
    _home: &str,
    config_dir: &Path,
    local_device: &str,
    current_user: &str,
    _from_user: &str,
    to_device: &str,
    to_ip: &str,
    to_user: &str,
    to_port: u16,
    connection_id: &str,
) -> Result<KeyActionResult, String> {
    // Check if device is reachable at its advertised IP
    if !is_reachable(to_ip, to_port) {
        return Ok(KeyActionResult::Error(format!(
            "Device {} unreachable at {}:{}",
            to_device, to_ip, to_port
        )));
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
        "Connecting via SSH to {}@{} ({}) on port {} to remove key...",
        to_user, to_ip, to_device, to_port
    ));
    crate::ui::log(
        "Note: If the remote device requires password authentication, you will be prompted for a password."
    );
    
    // Build the command for the remote device.
    // Options must come BEFORE the destination: everything after `user@host`
    // is sent to the remote as the command.
    let mut ssh_cmd = Command::new("ssh");
    if to_port != 22 {
        ssh_cmd.arg("-p").arg(to_port.to_string());
    }
    // Dial the beacon-advertised IP directly, not the device-name alias
    // (the alias lives in ~/.ssh/config, which regroup writes itself).
    ssh_cmd.arg(format!("{}@{}", to_user, to_ip));
    ssh_cmd
        .arg("regroup")
        .arg("receive-remove-key")
        .arg("--connection-id")
        .arg(connection_id);
    // Inherit the terminal so ssh can prompt for a password interactively
    // (see install_key_action for why the streams must not be captured).
    let status = ssh_cmd
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    
    match status {
        Ok(status) => {
            if status.success() {
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
