//! SSH server public key detection module
//!
//! This module detects the local SSH *server* host key fingerprint.
//! 
//! IMPORTANT: We need the server's host key, NOT the user's public key.
//! The server host key is what identifies a machine in the network and allows
//! SSH clients to verify they are connecting to the right machine.
//! 
//! Without a valid SSH server host key, the device should NOT announce itself
//! because it cannot be contacted via SSH.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::Write;

/// Get the local SSH server's public host key fingerprint.
/// 
/// This function attempts to find the SSH server host key by checking:
/// 1. System SSH host keys in /etc/ssh/ssh_host_*_key.pub
/// 2. Termux SSH host keys in $PREFIX/etc/ssh/ssh_host_*_key.pub
/// 3. Running ssh-keyscan on localhost (if SSH server is running)
/// 
/// Returns None if no SSH server host key can be found.
/// 
/// NOTE: If this returns None, the device should NOT announce itself via UDP
/// beacon because it cannot accept SSH connections.
pub fn get_ssh_server_fingerprint() -> Option<String> {
    // Try to read SSH server host keys from standard locations
    if let Some(fingerprint) = get_server_fingerprint_from_files() {
        return Some(fingerprint);
    }
    
    // Fallback: try ssh-keyscan on localhost
    // This works if the SSH server is running but we couldn't find the key files
    if let Some(fingerprint) = get_server_fingerprint_via_keyscan() {
        return Some(fingerprint);
    }
    
    // No SSH server host key found - device cannot accept SSH connections
    None
}

/// Read SSH server host key fingerprint from known file locations
/// 
/// Checks standard locations where SSH server host keys are stored:
/// - /etc/ssh/ssh_host_*_key.pub (standard Linux/Unix)
/// - $PREFIX/etc/ssh/ssh_host_*_key.pub (Termux Android)
fn get_server_fingerprint_from_files() -> Option<String> {
    // SSH server host key file patterns
    // These are the public keys corresponding to the private host keys
    let host_key_patterns = [
        "ssh_host_rsa_key.pub",
        "ssh_host_ecdsa_key.pub", 
        "ssh_host_ed25519_key.pub",
        "ssh_host_dsa_key.pub",
        // Some systems may have additional key types
        "ssh_host_rsa_key.pub", // Already listed, but for clarity
    ];
    
    // 1. Check standard system location: /etc/ssh/
    let etc_ssh_dir = PathBuf::from("/etc/ssh");
    if etc_ssh_dir.exists() {
        for pattern in &host_key_patterns {
            let key_path = etc_ssh_dir.join(pattern);
            if let Some(fingerprint) = read_host_key_fingerprint(&key_path) {
                return Some(fingerprint);
            }
        }
    }
    
    // 2. Check Termux location: $PREFIX/etc/ssh/
    // Termux installs OpenSSH with keys in its prefix directory
    if let Ok(prefix) = env::var("PREFIX") {
        let termux_ssh_dir = PathBuf::from(prefix).join("etc/ssh");
        if termux_ssh_dir.exists() {
            for pattern in &host_key_patterns {
                let key_path = termux_ssh_dir.join(pattern);
                if let Some(fingerprint) = read_host_key_fingerprint(&key_path) {
                    return Some(fingerprint);
                }
            }
        }
    }
    
    // 3. Check /usr/local/etc/ssh/ (BSD systems, homebrew, etc.)
    let usr_local_etc_ssh = PathBuf::from("/usr/local/etc/ssh");
    if usr_local_etc_ssh.exists() {
        for pattern in &host_key_patterns {
            let key_path = usr_local_etc_ssh.join(pattern);
            if let Some(fingerprint) = read_host_key_fingerprint(&key_path) {
                return Some(fingerprint);
            }
        }
    }
    
    None
}

/// Compute SHA256 fingerprint from a public key string (OpenSSH format)
/// 
/// The public key should be in the format: "key_type key_base64"
/// For example: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ..."
/// 
/// Uses ssh-keygen -lf to compute the fingerprint.
fn compute_fingerprint_from_public_key(public_key: &str) -> Option<String> {
    use std::process::Stdio;
    
    // ssh-keygen -lf expects the key to be passed via stdin or as a file
    // We'll pass it via stdin
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };
    
    // Write the public key to ssh-keygen's stdin
    if let Some(mut stdin) = child.stdin.take() {
        // ssh-keygen expects the key in the format that ssh-keyscan produces
        // which is: hostname key_type key_base64
        // But we only have key_type key_base64, so we need to add a dummy hostname
        let key_with_hostname = format!("dummy {}", public_key);
        if let Err(_) = stdin.write_all(key_with_hostname.as_bytes()) {
            return None;
        }
    }
    
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return None,
    };
    
    if !output.status.success() {
        return None;
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        for field in line.split_whitespace() {
            if field.starts_with("SHA256:") {
                return Some(field.to_string());
            }
        }
    }
    
    // Fallback to MD5
    for line in stdout.lines() {
        for field in line.split_whitespace() {
            if field.starts_with("MD5:") {
                return Some(field.to_string());
            }
        }
    }
    
    None
}

/// Use ssh-keyscan to get the server host key fingerprint from localhost
/// 
/// This is a fallback method that queries the running SSH server
/// on localhost to get its host key. This only works if:
/// - An SSH server is running on this machine
/// - It's listening on 127.0.0.1
/// - We can connect to it (no firewall blocking)
fn get_server_fingerprint_via_keyscan() -> Option<String> {
    // Try to scan localhost for SSH host keys
    let scan = Command::new("ssh-keyscan")
        .arg("-T")  // Timeout for connection
        .arg("3")    // 3 seconds timeout
        .arg("127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    
    let raw = match scan {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => o.stdout,
        _ => return None,
    };
    
    // Parse the ssh-keyscan output to get fingerprints
    // ssh-keyscan output format: hostname key_type key_base64
    // We need to compute the fingerprint from the key
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")        // Compute fingerprint
        .arg("-")          // Read from stdin
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };
    
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&raw);
    }
    
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return None,
    };
    
    if !out.status.success() {
        return None;
    }
    
    // Parse the fingerprint from ssh-keygen output
    // Format: "bits SHA256:... hostname (key_type)"
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        for field in &fields {
            if field.starts_with("SHA256:") {
                return Some(field.to_string());
            }
        }
    }
    
    None
}

/// Read an SSH host key file and return its fingerprint
/// 
/// Uses ssh-keygen -lf to compute the SHA256 fingerprint of the key.
/// Returns None if the file doesn't exist, can't be read, or ssh-keygen fails.
fn read_host_key_fingerprint(key_path: &PathBuf) -> Option<String> {
    // Check if the file exists and is readable
    if !key_path.exists() {
        return None;
    }
    
    // Verify it's actually a file (not a directory)
    if !key_path.is_file() {
        return None;
    }
    
    // Use ssh-keygen to compute the fingerprint
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")           // Compute fingerprint
        .arg(key_path)        // Path to the public key file
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return None,
    };
    
    if !output.status.success() {
        return None;
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    
    if lines.is_empty() {
        return None;
    }
    
    // Parse the fingerprint from ssh-keygen output
    // Format: "bits SHA256:... comment (type)"
    // We want the SHA256 (or MD5 as fallback) fingerprint
    let fields: Vec<&str> = lines[0].split_whitespace().collect();
    for field in &fields {
        if field.starts_with("SHA256:") {
            return Some(field.to_string());
        }
    }
    
    // Fallback to MD5 if SHA256 not found (older systems)
    for field in &fields {
        if field.starts_with("MD5:") {
            return Some(field.to_string());
        }
    }
    
    None
}

/// Result type for host key verification
#[derive(Debug, Clone, PartialEq)]
pub enum HostKeyVerification {
    /// Key verified and added successfully
    Success,
    /// Host is offline or unreachable
    HostUnreachable,
    /// Fingerprint mismatch - POTENTIAL SECURITY THREAT
    FingerprintMismatch { expected: String, actual: String },
    /// Error writing to known_hosts file
    WriteError,
}

/// Add a host's public key to ~/.ssh/known_hosts for automatic verification
/// 
/// This function attempts to:
/// 1. Scan the remote host's SSH keys using ssh-keyscan
/// 2. Append the keys to ~/.ssh/known_hosts
/// 
/// This allows SSH clients to automatically verify the host key when connecting,
/// preventing man-in-the-middle attacks.
/// 
/// Returns true if the key was successfully added to known_hosts.
pub fn add_to_known_hosts(hostname: &str, ip: &str, port: u16) -> bool {
    // Simply add to known_hosts without verification (for backward compatibility)
    add_to_known_hosts_inner(hostname, ip, port).is_ok()
}

/// Internal function to add host to known_hosts
fn add_to_known_hosts_inner(hostname: &str, ip: &str, port: u16) -> std::io::Result<()> {
    // Try to get the host's public keys via ssh-keyscan
    let keyscan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("-p")
        .arg(port.to_string())
        .arg(ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    
    let keyscan_output = match keyscan {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => o.stdout,
        _ => return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Host unreachable")),
    };
    
    // Parse the ssh-keyscan output to extract host keys
    // Format: hostname key_type key_base64
    let keyscan_str = match String::from_utf8(keyscan_output) {
        Ok(s) => s,
        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid key data")),
    };
    
    // Get the HOME directory
    let home_dir = match env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set")),
    };
    
    let known_hosts_path = home_dir.join(".ssh").join("known_hosts");
    
    // Ensure ~/.ssh directory exists
    let ssh_dir = home_dir.join(".ssh");
    if let Err(_) = fs::create_dir_all(&ssh_dir) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Cannot create .ssh directory"));
    }
    
    // Ensure known_hosts file exists
    if !known_hosts_path.exists() {
        if let Err(_) = fs::write(&known_hosts_path, "") {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Cannot create known_hosts"));
        }
        // Set permissions to 600 for security
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(_) = fs::set_permissions(&known_hosts_path, fs::Permissions::from_mode(0o600)) {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Cannot set permissions"));
            }
        }
    }
    
    // Check if this host is already in known_hosts
    // We check by hostname and IP to avoid duplicates
    if let Ok(existing) = fs::read_to_string(&known_hosts_path) {
        if existing.contains(hostname) || existing.contains(ip) {
            // Already in known_hosts, nothing to do
            return Ok(());
        }
    }
    
    // Append the new keys to known_hosts
    // Format: hostname,ip key_type key_base64
    let entry = format!("# Added by regroup\n{}", keyscan_str.trim());
    
    if let Err(e) = fs::OpenOptions::new()
        .append(true)
        .open(&known_hosts_path)
        .and_then(|mut f| writeln!(f, "{}", entry)) {
        return Err(e);
    }
    
    Ok(())
}

/// Verify that a remote host's SSH key fingerprint matches the expected one.
/// 
/// This is a CRITICAL security check. When a device announces itself via UDP beacon,
/// it provides its SSH server host key fingerprint. When we add the device, we should
/// verify that the actual SSH server on that IP has the same fingerprint.
/// 
/// If the fingerprints don't match, it could indicate:
/// - A man-in-the-middle attack
/// - The device has been replaced
/// - The device's SSH keys were regenerated
/// - Network misconfiguration
/// 
/// This function:
/// 1. Scans the remote host's SSH keys using ssh-keyscan
/// 2. Computes the SHA256 fingerprint of the first key
/// 3. Compares it with the expected fingerprint
/// 4. If they match, adds the key to ~/.ssh/known_hosts
/// 
/// Returns:
/// - HostKeyVerification::Success if verified and added
/// - HostKeyVerification::HostUnreachable if the host is offline
/// - HostKeyVerification::FingerprintMismatch if the fingerprints don't match (SECURITY ALERT!)
/// - HostKeyVerification::WriteError if we couldn't write to known_hosts
pub fn verify_and_add_to_known_hosts(
    hostname: &str,
    ip: &str,
    port: u16,
    expected_fingerprint: &str,
) -> HostKeyVerification {
    // Try to get the host's public keys via ssh-keyscan
    let keyscan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("-p")
        .arg(port.to_string())
        .arg(ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    
    let keyscan_output = match keyscan {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => o.stdout,
        _ => return HostKeyVerification::HostUnreachable,
    };
    
    // Parse the ssh-keyscan output to get the public key
    // Format: hostname key_type key_base64
    let keyscan_str = match String::from_utf8(keyscan_output.clone()) {
        Ok(s) => s,
        Err(_) => return HostKeyVerification::HostUnreachable,
    };
    
    // Extract just the key part (not the hostname prefix)
    // ssh-keyscan output: "hostname key_type key_base64\n"
    let key_line = match keyscan_str.lines().next() {
        Some(line) => line,
        None => return HostKeyVerification::HostUnreachable,
    };
    
    // The line format is: hostname key_type key_base64
    // We need to extract just "key_type key_base64" for fingerprint computation
    let key_parts: Vec<&str> = key_line.split_whitespace().collect();
    if key_parts.len() < 3 {
        return HostKeyVerification::HostUnreachable;
    }
    
    // Reconstruct the key in the format: "key_type key_base64"
    // This is what ssh-keygen -lf expects
    let public_key = key_parts[1..].join(" ");
    
    // Compute the fingerprint of the retrieved key
    let actual_fingerprint = match compute_fingerprint_from_public_key(&public_key) {
        Some(fp) => fp,
        None => return HostKeyVerification::HostUnreachable,
    };
    
    // Compare with expected fingerprint
    // Both should be in format "SHA256:..." or "MD5:..."
    if actual_fingerprint != expected_fingerprint {
        return HostKeyVerification::FingerprintMismatch {
            expected: expected_fingerprint.to_string(),
            actual: actual_fingerprint,
        };
    }
    
    // Fingerprint matches! Add to known_hosts
    match add_to_known_hosts_inner(hostname, ip, port) {
        Ok(_) => HostKeyVerification::Success,
        Err(_) => HostKeyVerification::WriteError,
    }
}
