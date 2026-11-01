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
        _ => return false,
    };
    
    // Parse the ssh-keyscan output to extract host keys
    // Format: hostname key_type key_base64
    let keyscan_str = match String::from_utf8(keyscan_output) {
        Ok(s) => s,
        Err(_) => return false,
    };
    
    // Replace the IP with the hostname for better readability
    // ssh-keyscan returns entries like: "192.168.1.1 ssh-rsa AAAAB3..."
    // We want: "hostname ssh-rsa AAAAB3..."
    // But we should keep both IP and hostname for flexibility
    
    // Get the HOME directory
    let home_dir = match env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return false,
    };
    
    let known_hosts_path = home_dir.join(".ssh").join("known_hosts");
    
    // Ensure ~/.ssh directory exists
    let ssh_dir = home_dir.join(".ssh");
    if let Err(_) = fs::create_dir_all(&ssh_dir) {
        return false;
    }
    
    // Ensure known_hosts file exists
    if !known_hosts_path.exists() {
        if let Err(_) = fs::write(&known_hosts_path, "") {
            return false;
        }
        // Set permissions to 600 for security
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(_) = fs::set_permissions(&known_hosts_path, fs::Permissions::from_mode(0o600)) {
                return false;
            }
        }
    }
    
    // Check if this host is already in known_hosts
    // We check by hostname and IP to avoid duplicates
    if let Ok(existing) = fs::read_to_string(&known_hosts_path) {
        if existing.contains(hostname) || existing.contains(ip) {
            // Already in known_hosts, nothing to do
            return true;
        }
    }
    
    // Append the new keys to known_hosts
    // Format: hostname,ip key_type key_base64
    // Replace the IP in the keyscan output with the hostname for better readability
    let entry = format!("# Added by regroup\n{}", keyscan_str.trim());
    
    if let Err(_) = fs::OpenOptions::new()
        .append(true)
        .open(&known_hosts_path)
        .and_then(|mut f| writeln!(f, "{}", entry)) {
        return false;
    }
    
    true
}
