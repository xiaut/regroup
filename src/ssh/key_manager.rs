//! SSH key management for regroup connections
//!
//! This module handles:
//! - Generating SSH key pairs for regroup connections
//! - Installing public keys on remote machines
//! - Removing public keys from remote machines
//! - Managing key files with regroup-specific naming

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::env::VarError;

use crate::config::Connection;

/// Default SSH directory
const SSH_DIR: &str = ".ssh";

/// Prefix for regroup-generated key files
const REGROUP_KEY_PREFIX: &str = "id_regroup_";

/// Generates a new SSH key pair for a regroup connection
/// Returns (private_key_path, public_key, public_key_content)
pub fn generate_key_pair(
    from: &str,
    to: &str,
    key_type: &str,
) -> io::Result<(PathBuf, String, String)> {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(VarError::NotPresent) => return Err(io::Error::new(io::ErrorKind::NotFound, "HOME environment variable not set")),
        Err(VarError::NotUnicode(_)) => return Err(io::Error::new(io::ErrorKind::InvalidData, "HOME contains invalid Unicode")),
    };
    let ssh_dir = PathBuf::from(home).join(SSH_DIR);
    
    // Create .ssh directory if it doesn't exist
    fs::create_dir_all(&ssh_dir)?;
    
    // Generate a unique filename
    let sanitized_from = from.replace("@", "_").replace("/", "_");
    let sanitized_to = to.replace("@", "_").replace("/", "_");
    let key_filename = format!("{} {}_to_{}", REGROUP_KEY_PREFIX, sanitized_from, sanitized_to);
    
    let private_key_path = ssh_dir.join(key_filename);
    let public_key_path = private_key_path.with_extension("pub");
    
    // Generate the key pair
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-t").arg(key_type)
        .arg("-f").arg(&private_key_path)
        .arg("-N").arg("")  // No passphrase
        .arg("-q")          // Quiet mode
        .arg("-C").arg(format!("regroup: {} -> {}", from, to));
    
    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(io::ErrorKind::Other, format!("ssh-keygen failed: {}", stderr)));
    }
    
    // Read the public key
    let public_key = fs::read_to_string(&public_key_path)?;
    let public_key_line = public_key.lines().next().unwrap_or("").trim().to_string();
    
    // Set proper permissions
    set_key_permissions(&private_key_path)?;
    set_key_permissions(&public_key_path)?;
    
    Ok((private_key_path, public_key_line, public_key))
}

/// Sets proper permissions for SSH key files (600 for private, 644 for public)
fn set_key_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        
        let metadata = fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        
        if path.extension().and_then(|s| s.to_str()) == Some("") {
            // Private key: 600
            permissions.set_mode(0o600);
        } else {
            // Public key: 644
            permissions.set_mode(0o644);
        }
        
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// Installs a public key on a remote machine for a specific user
pub fn install_public_key(
    hostname: &str,
    port: u16,
    user: &str,
    public_key: &str,
) -> io::Result<String> {
    // Verify the public key format
    if public_key.split_whitespace().count() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid public key format"));
    }
    
    // Build the SSH command to append the key
    let cmd = format!(
        "ssh -p {} -o StrictHostKeyChecking=no -o BatchMode=no {}@{} \
         'mkdir -p ~/.ssh 2>/dev/null; \
          chmod 700 ~/.ssh 2>/dev/null; \
          grep -qxF '\"{}\"' ~/.ssh/authorized_keys 2>/dev/null || echo '\"{}\"' >> ~/.ssh/authorized_keys; \
          chmod 600 ~/.ssh/authorized_keys 2>/dev/null; \
          echo OK'",
        port, user, hostname, public_key, public_key
    );
    
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to install key: {}", stderr)));
    }
    
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Removes a public key from a remote machine
pub fn remove_public_key(
    hostname: &str,
    port: u16,
    user: &str,
    public_key: &str,
) -> io::Result<String> {
    // Build the SSH command to remove the key
    let cmd = format!(
        "ssh -p {} -o StrictHostKeyChecking=no -o BatchMode=no {}@{} \
         'grep -vxF '\"{}\"' ~/.ssh/authorized_keys > ~/.ssh/authorized_keys.tmp 2>/dev/null; \
          mv ~/.ssh/authorized_keys.tmp ~/.ssh/authorized_keys 2>/dev/null; \
          chmod 600 ~/.ssh/authorized_keys 2>/dev/null; \
          echo OK'",
        port, user, hostname, public_key
    );
    
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to remove key: {}", stderr)));
    }
    
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Deletes a regroup-generated private key file
pub fn delete_private_key(private_key_path: &Path) -> io::Result<()> {
    if private_key_path.exists() {
        fs::remove_file(private_key_path)?;
        
        // Also remove the public key file
        let public_key_path = private_key_path.with_extension("pub");
        if public_key_path.exists() {
            fs::remove_file(public_key_path)?;
        }
    }
    Ok(())
}

/// Lists all regroup-generated key files
pub fn list_regroup_keys() -> io::Result<Vec<PathBuf>> {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(VarError::NotPresent) => return Err(io::Error::new(io::ErrorKind::NotFound, "HOME environment variable not set")),
        Err(VarError::NotUnicode(_)) => return Err(io::Error::new(io::ErrorKind::InvalidData, "HOME contains invalid Unicode")),
    };
    let ssh_dir = PathBuf::from(home).join(SSH_DIR);
    
    if !ssh_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut keys = Vec::new();
    for entry in fs::read_dir(&ssh_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            if filename.starts_with(REGROUP_KEY_PREFIX) {
                keys.push(path);
            }
        }
    }
    
    Ok(keys)
}

/// Creates a connection and generates the SSH key pair
pub fn create_connection(
    config_dir: &Path,
    local_machine: &str,
    local_user: &str,
    target_machine: &str,
    target_user: &str,
    key_type: &str,
) -> io::Result<Connection> {

    
    // Generate the key pair
    let from = format!("{}@{}", local_user, local_machine);
    let to = format!("{}@{}", target_user, target_machine);
    
    let (private_key_path, public_key, _) = generate_key_pair(&from, &to, key_type)?;
    
    // Create the connection
    let created_at = chrono::Utc::now().to_rfc3339();
    
    let connection = Connection::new(
        &from,
        &to,
        &public_key,
        private_key_path.to_string_lossy().as_ref(),
        &created_at,
    );
    
    // Save the connection files
    crate::config::connection::save_source_connection(config_dir, local_machine, &connection)?;
    crate::config::connection::save_target_connection(config_dir, target_machine, &connection)?;
    
    Ok(connection)
}

/// Removes a connection and its associated keys
pub fn remove_connection(
    config_dir: &Path,
    local_machine: &str,
    target_machine: &str,
    connection: &Connection,
) -> io::Result<()> {
    // Delete the connection files
    crate::config::connection::delete_source_connection(config_dir, local_machine, connection)?;
    crate::config::connection::delete_target_connection(config_dir, target_machine, connection)?;
    
    // Delete the private key file
    let private_key_path = PathBuf::from(&connection.private_key_path);
    delete_private_key(&private_key_path)?;
    
    // Remove the public key from the target machine
    let target = connection.to.clone();
    if let Some((target_user, _)) = target.split_once('@') {
        // We need the target machine's port from its config
        // For now, we'll use the default port 22
        // In a real implementation, we'd look this up
        let _ = remove_public_key(target_machine, 22, target_user, &connection.public_key);
    }
    
    Ok(())
}

/// Lists all connections for a target machine
pub fn list_connections_for_target(config_dir: &Path, target_machine: &str) -> io::Result<Vec<Connection>> {
    use crate::config::connection;
    
    let connections_dir = config_dir.join(connection::CONNECTIONS_DIR).join(target_machine);
    
    if !connections_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut connections = Vec::new();
    for entry in fs::read_dir(&connections_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(conn) = serde_json::from_str::<Connection>(&content) {
                    connections.push(conn);
                }
            }
        }
    }
    
    Ok(connections)
}
