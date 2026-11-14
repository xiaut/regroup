//! Key management utilities for SSH connection handling
//!
//! This module provides functions for generating, installing, and removing
//! SSH key pairs for regroup connections.

#[allow(dead_code)]
use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::config::{CONFIG_DIR, GIT_DIR};
use crate::git::sync::git_add_and_commit_file;

/// Directory for SSH keys
const SSH_DIR: &str = ".ssh";

/// Generate a new SSH key pair for a connection
#[allow(dead_code)]
///
/// Creates an ed25519 key pair with empty passphrase and a descriptive comment.
/// Key file name: `id_regroup_<from_user>@<to_device>`
///
/// Returns the path to the private key, or an error.
pub fn generate_key_pair(from_user: &str, to_device: &str) -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let ssh_dir = PathBuf::from(&home).join(SSH_DIR);
    
    // Create ~/.ssh if it doesn't exist
    if !ssh_dir.exists() {
        fs::create_dir(&ssh_dir).map_err(|e| format!("Cannot create ~/.ssh: {}", e))?;
        let _ = fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700));
    }
    
    let key_name = format!("id_regroup_{}@{}", from_user, to_device);
    let key_path = ssh_dir.join(key_name);
    
    // Check if key already exists
    if key_path.exists() {
        return Ok(key_path);
    }
    
    // Generate new ed25519 key with empty passphrase
    let output = Command::new("ssh-keygen")
        .arg("-t")
        .arg("ed25519")
        .arg("-N")
        .arg("")  // Empty passphrase
        .arg("-f")
        .arg(&key_path)
        .arg("-C")
        .arg(&format!("regroup:{}->{}@{}", from_user, from_user, to_device))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Cannot run ssh-keygen: {}", e))?;
    
    // Log stderr if present
    if !output.stderr.is_empty() {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        for line in stderr_str.lines() {
            if !line.is_empty() {
                crate::ui::log(&format!("ssh-keygen: {}", line));
            }
        }
    }
    
    if !output.status.success() {
        return Err(format!("ssh-keygen failed: {}", output.status));
    }
    
    // Set proper permissions
    let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    let pub_path = key_path.with_extension("pub");
    if pub_path.exists() {
        let _ = fs::set_permissions(&pub_path, fs::Permissions::from_mode(0o644));
    }
    
    Ok(key_path)
}

/// Get the public key content from a key file
#[allow(dead_code)]
pub fn get_public_key(key_path: &Path) -> Result<String, String> {
    let pub_path = key_path.with_extension("pub");
    fs::read_to_string(&pub_path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("Cannot read public key: {}", e))
}

/// Install a public key in authorized_keys on the local device
///
/// Adds the key with a descriptive comment for identification.
pub fn install_authorized_key(public_key: &str, connection_id: &str) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let authorized_keys_path = PathBuf::from(&home).join(SSH_DIR).join("authorized_keys");
    
    // Create authorized_keys if it doesn't exist
    if !authorized_keys_path.exists() {
        fs::write(&authorized_keys_path, "")
            .map_err(|e| format!("Cannot create authorized_keys: {}", e))?;
        let _ = fs::set_permissions(&authorized_keys_path, fs::Permissions::from_mode(0o600));
    }
    
    let comment = format!("regroup:{}", connection_id);
    let line = format!("{} {}", public_key, comment);
    
    // Check if this key is already installed
    let content = fs::read_to_string(&authorized_keys_path)
        .map_err(|e| format!("Cannot read authorized_keys: {}", e))?;
    
    if content.lines().any(|l| l.trim() == line) {
        return Err(format!(
            "Key already installed in authorized_keys for connection {}",
            connection_id
        ));
    }
    
    // Append the new key
    fs::write(&authorized_keys_path, format!("{}\n{}", content, line))
        .map_err(|e| format!("Cannot write authorized_keys: {}", e))?;
    
    Ok(())
}

/// Remove a public key from authorized_keys
///
/// Identifies the key by its content (first field of the line).
pub fn remove_authorized_key(public_key: &str) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let authorized_keys_path = PathBuf::from(&home).join(SSH_DIR).join("authorized_keys");
    
    if !authorized_keys_path.exists() {
        return Ok(()); // No authorized_keys file, nothing to remove
    }
    
    let content = fs::read_to_string(&authorized_keys_path)
        .map_err(|e| format!("Cannot read authorized_keys: {}", e))?;
    
    // Remove all lines that start with this public key
    let new_content: String = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return true; // Keep empty lines
            }
            let first_word = trimmed.split_whitespace().next().unwrap_or("");
            first_word != public_key
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    if new_content != content {
        fs::write(&authorized_keys_path, new_content)
            .map_err(|e| format!("Cannot write authorized_keys: {}", e))?;
    }
    
    Ok(())
}

/// Check that a device is reachable at the TCP level, at the IP it advertised
/// in its beacon.
///
/// Two deliberate choices:
/// - The device NAME is never resolved here — no DNS, no ~/.ssh/config alias.
///   regroup writes that alias itself, so resolving through it would be
///   circular and would break on any network where the name does not resolve.
///   The beacon IP is the authoritative transport address (it is checked
///   against the UDP packet's source address when the beacon is received).
/// - Authentication is NOT tested: installing or removing a key may
///   legitimately rely on password auth (the key is not authorized yet), and
///   an `ssh -o BatchMode=yes ... true` probe would refuse exactly that case
///   and report the device as unreachable.
#[allow(dead_code)]
pub fn is_reachable(ip: &str, port: u16) -> bool {
    crate::ui::log(&format!("Checking if {}:{} is reachable...", ip, port));
    let addrs: Vec<_> = match (ip, port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            crate::ui::log(&format!("cannot resolve {}: {}", ip, e));
            return false;
        }
    };
    for addr in &addrs {
        if TcpStream::connect_timeout(addr, Duration::from_secs(5)).is_ok() {
            return true;
        }
    }
    crate::ui::log(&format!("connect to {}:{} failed", ip, port));
    false
}

/// Get the config directory path
pub fn get_config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(&home).join(CONFIG_DIR).join(GIT_DIR)
}

/// Commit changes to the git repository
pub fn commit_changes(config_dir: &Path, message: &str) -> Result<(), String> {
    // We need to add all changes in the connections directory
    let connections_dir = config_dir.join("devices").join(get_local_device_name()).join("connections");
    
    // Use git_add_and_commit_file for the directory (which will add all changes inside)
    git_add_and_commit_file(config_dir, &connections_dir, message)
        .map_err(|e| format!("Git commit failed: {}", e))?;
    
    Ok(())
}

/// Parse connection ID to extract components
///
/// Expected format: "from_user->to_user@to_device"
pub fn parse_connection_id(connection_id: &str) -> Result<(String, String, String), String> {
    // Split by '->' first
    let parts: Vec<&str> = connection_id.splitn(2, "->").collect();
    if parts.len() != 2 {
        return Err(format!("Invalid connection ID format: {}", connection_id));
    }
    
    let from_part = parts[0].trim();
    let to_part = parts[1].trim();
    
    // Split to_part by '@' 
    let to_parts: Vec<&str> = to_part.splitn(2, '@').collect();
    if to_parts.len() != 2 {
        return Err(format!("Invalid connection ID format (missing @): {}", connection_id));
    }
    
    Ok((
        from_part.to_string(),
        to_parts[0].to_string(),
        to_parts[1].to_string(),
    ))
}

/// Build connection ID from components
#[allow(dead_code)]
pub fn build_connection_id(from_user: &str, to_user: &str, to_device: &str) -> String {
    format!("{}->{}@{}", from_user, to_user, to_device)
}

/// Get the local device name
pub fn get_local_device_name() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| {
        // Fallback: try to get from /etc/hostname
        fs::read_to_string("/etc/hostname").unwrap_or_else(|_| "unknown".to_string())
    })
}

/// Get the current user
pub fn get_current_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| {
        std::env::var("LOGNAME").unwrap_or_else(|_| "unknown".to_string())
    })
}

