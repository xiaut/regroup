//! SSH server public key detection and verification module
//!
//! This module:
//! - detects the local SSH *server* host public key (the machine's identity)
//! - computes SHA256 fingerprints of host keys
//! - verifies that the host key announced in a beacon is really served by the
//!   SSH server at the announced IP, and pins it in ~/.ssh/known_hosts
//!
//! IMPORTANT: We need the server's host key, NOT the user's public key.
//! The server host key is what identifies a machine in the network and allows
//! SSH clients to verify they are connecting to the right machine.
//!
//! Without a valid SSH server host key, the device should NOT announce itself
//! because it cannot be contacted via SSH.

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::keys::{SshFingerprint, SshPublicKey};

// ============================================================
// Local host key detection
// ============================================================

/// Get the local SSH server's public host key in OpenSSH format.
///
/// This function reads the full public key (e.g., "ssh-rsa AAAAB3NzaC1yc2E..." or
/// "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...") from the server's host key files.
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
pub fn get_ssh_server_public_key() -> Option<SshPublicKey> {
    // Try to read SSH server host keys from standard locations
    if let Some(public_key) = get_server_public_key_from_files() {
        return Some(public_key);
    }

    // Fallback: try ssh-keyscan on localhost
    if let Some(public_key) = get_server_public_key_via_keyscan() {
        return Some(public_key);
    }

    None
}

/// Read SSH server host key public key from known file locations
///
/// Checks standard locations where SSH server host keys are stored:
/// - /etc/ssh/ssh_host_*_key.pub (standard Linux/Unix)
/// - $PREFIX/etc/ssh/ssh_host_*_key.pub (Termux Android)
fn get_server_public_key_from_files() -> Option<SshPublicKey> {
    // SSH server host key file patterns
    let host_key_patterns = [
        "ssh_host_rsa_key.pub",
        "ssh_host_ecdsa_key.pub",
        "ssh_host_ed25519_key.pub",
        "ssh_host_dsa_key.pub",
    ];

    // 1. Check standard system location: /etc/ssh/
    let etc_ssh_dir = PathBuf::from("/etc/ssh");
    if etc_ssh_dir.exists() {
        for pattern in &host_key_patterns {
            let key_path = etc_ssh_dir.join(pattern);
            if let Some(public_key) = read_host_key(&key_path) {
                return Some(public_key);
            }
        }
    }

    // 2. Check Termux location: $PREFIX/etc/ssh/
    if let Ok(prefix) = env::var("PREFIX") {
        let termux_ssh_dir = PathBuf::from(prefix).join("etc/ssh");
        if termux_ssh_dir.exists() {
            for pattern in &host_key_patterns {
                let key_path = termux_ssh_dir.join(pattern);
                if let Some(public_key) = read_host_key(&key_path) {
                    return Some(public_key);
                }
            }
        }
    }

    // 3. Check /usr/local/etc/ssh/
    let usr_local_etc_ssh = PathBuf::from("/usr/local/etc/ssh");
    if usr_local_etc_ssh.exists() {
        for pattern in &host_key_patterns {
            let key_path = usr_local_etc_ssh.join(pattern);
            if let Some(public_key) = read_host_key(&key_path) {
                return Some(public_key);
            }
        }
    }

    None
}

/// Use ssh-keyscan to get the server host public key from localhost
fn get_server_public_key_via_keyscan() -> Option<SshPublicKey> {
    // Try to scan localhost for SSH host keys
    crate::ui::log("Scanning localhost for SSH host keys (may prompt for password)...");
    // Show stderr for visibility
    let scan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let output = match scan {
        Ok(o) => o,
        Err(_) => return None,
    };

    // Log stderr if present
    if !output.stderr.is_empty() {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        for line in stderr_str.lines() {
            if !line.is_empty() {
                crate::ui::log(&format!("ssh-keyscan: {}", line));
            }
        }
    }

    let raw = if output.status.success() && !output.stdout.is_empty() {
        output.stdout
    } else {
        return None;
    };

    let output_str = String::from_utf8_lossy(&raw);
    // ssh-keyscan output: "127.0.0.1 ssh-rsa AAAAB3NzaC1..."
    // We want to extract just "ssh-rsa AAAAB3NzaC1..." (the key_type key_base64 part)
    for line in output_str.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            // The key is parts[1] and parts[2] (key_type and key_base64)
            // Take only first 2 parts of the key to avoid including comments
            let key = parts[1..].join(" ");
            if let Some(normalized) = normalize_public_key(&key) {
                return Some(SshPublicKey::from(normalized));
            }
        }
    }
    None
}

/// Read the content of an SSH host key public key file
/// Returns the full key in OpenSSH format (key_type key_base64)
fn read_host_key(key_path: &PathBuf) -> Option<SshPublicKey> {
    if !key_path.exists() || !key_path.is_file() {
        return None;
    }

    match fs::read_to_string(key_path) {
        Ok(content) => {
            // The file should contain just the key in OpenSSH format
            // Remove any comments or trailing whitespace
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                // Normalize to extract just key_type and key_base64
                if let Some(normalized) = normalize_public_key(&trimmed) {
                    return Some(SshPublicKey::from(normalized));
                }
            }
            None
        }
        Err(_) => None,
    }
}

// ============================================================
// Fingerprint computation
// ============================================================

/// Normalize an SSH public key string to extract just key_type and key_base64.
/// Removes any hostname prefix or comment that may be present.
/// Returns None if the key doesn't appear to be valid.
fn normalize_public_key(public_key: &str) -> Option<String> {
    // Known SSH key types for detection (including RSA signature algorithms)
    let known_key_types = ["ssh-rsa", "rsa-sha2-256", "rsa-sha2-512", "rsa-sha2-256-cert-v01@openssh.com", "rsa-sha2-512-cert-v01@openssh.com",
        "ssh-dss", "ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com", "sk-ecdsa-sha2-nistp256@openssh.com"];
    
    let parts: Vec<&str> = public_key.split_whitespace().collect();
    
    // Try to find a known key type in the parts
    let mut key_start = 0;
    for (i, part) in parts.iter().enumerate() {
        if known_key_types.contains(part) {
            key_start = i;
            break;
        }
    }
    
    // Ensure we have at least 2 parts starting from key_start
    // If no known key type found, start from beginning
    // If key_start is too close to end, start from beginning
    if key_start <= parts.len() - 2 {
        Some(parts[key_start..key_start+2].join(" "))
    } else if parts.len() >= 2 {
        // Fallback: use first two parts
        Some(parts[0..2].join(" "))
    } else {
        None
    }
}

/// Compute SHA256 fingerprint from a public key string (OpenSSH format)
///
/// The public key should be in the format: "key_type key_base64"
/// For example: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ..."
///
/// Uses ssh-keygen -lf to compute the fingerprint.
pub fn compute_fingerprint_from_public_key(public_key: &str) -> Option<SshFingerprint> {
    // ssh-keygen -lf expects the key to be passed via stdin or as a file
    // We'll pass it via stdin
    // Use ssh-keygen to compute fingerprint
    // Show stderr for visibility
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };

    // Normalize: extract just key_type and key_base64 (ignore hostname/comment if present)
    let key_only = match normalize_public_key(public_key) {
        Some(k) => k,
        None => return None,
    };

    // Write the public key to ssh-keygen's stdin
    if let Some(mut stdin) = child.stdin.take() {
        // ssh-keygen expects the key in the format that ssh-keyscan produces
        // which is: hostname key_type key_base64
        // We add a dummy hostname to the normalized key
        // The line must end with a newline for ssh-keygen to recognize it
        let key_with_hostname = format!("dummy {}\n", key_only);
        if stdin.write_all(key_with_hostname.as_bytes()).is_err() {
            return None;
        }
        // Explicitly drop stdin to close the pipe and signal EOF to ssh-keygen
        drop(stdin);
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return None,
    };

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
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        for field in line.split_whitespace() {
            if field.starts_with("SHA256:") {
                return Some(SshFingerprint::from(field));
            }
        }
    }

    // Fallback to MD5 (older systems)
    for line in stdout.lines() {
        for field in line.split_whitespace() {
            if field.starts_with("MD5:") {
                return Some(SshFingerprint::from(field));
            }
        }
    }

    None
}

// ============================================================
// Host key verification (anti-MITM)
// ============================================================

/// Result of a host key verification
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

/// Fetch the host keys offered by the SSH server at ip:port via ssh-keyscan.
/// Returns the raw keyscan output (lines of "host keytype key"), or an error
/// if the host is unreachable.
fn keyscan(ip: &str, port: u16) -> std::io::Result<String> {
    // Fetch host keys via ssh-keyscan
    crate::ui::log(&format!(
        "Fetching SSH host keys from {}:{} (may prompt for password)...",
        ip, port
    ));
    // Show stderr for visibility
    let output = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("-p")
        .arg(port.to_string())
        .arg(ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(o) => {
            // Log stderr if present
            if !o.stderr.is_empty() {
                let stderr_str = String::from_utf8_lossy(&o.stderr);
                for line in stderr_str.lines() {
                    if !line.is_empty() {
                        crate::ui::log(&format!("ssh-keyscan: {}", line));
                    }
                }
            }
            if o.status.success() && !o.stdout.is_empty() {
                String::from_utf8(o.stdout)
                    .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid key data"))
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "host unreachable",
                ))
            }
        }
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "host unreachable",
        )),
    }
}

/// Check whether ~/.ssh/known_hosts already has an entry for ip:port.
/// Entries are matched exactly on the host pattern ("ip" for the default
/// port, "[ip]:port" otherwise), never by substring.
fn known_hosts_has_ip_entry(content: &str, ip: &str, port: u16) -> bool {
    let ip_entry = if port == 22 {
        ip.to_string()
    } else {
        format!("[{}]:{}", ip, port)
    };
    content.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|patterns| patterns.split(',').any(|p| p == ip_entry))
    })
}

/// Append host keys (ssh-keyscan output) to ~/.ssh/known_hosts, keyed by ip.
/// Skips the write when an entry for this ip:port already exists.
fn append_known_hosts_entries(ip: &str, port: u16, keyscan_str: &str) -> std::io::Result<()> {
    // Get the HOME directory
    let home_dir = match env::var("HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME not set",
            ));
        }
    };

    let ssh_dir = home_dir.join(".ssh");
    let known_hosts_path = ssh_dir.join("known_hosts");

    // Ensure ~/.ssh directory exists
    fs::create_dir_all(&ssh_dir)?;

    // Ensure known_hosts file exists with restrictive permissions
    if !known_hosts_path.exists() {
        fs::write(&known_hosts_path, "")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&known_hosts_path, fs::Permissions::from_mode(0o600))?;
        }
    }

    // Avoid duplicates: only add if this ip:port is not pinned yet
    if let Ok(existing) = fs::read_to_string(&known_hosts_path)
        && known_hosts_has_ip_entry(&existing, ip, port)
    {
        return Ok(());
    }

    // Append the new keys to known_hosts
    let entry = format!("# Added by regroup\n{}", keyscan_str.trim());

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&known_hosts_path)?;
    writeln!(file, "{}", entry)
}

/// Remove ~/.ssh/known_hosts entries for `ip`, whatever the port ("ip" and
/// "[ip]:port" patterns), including the "# Added by regroup" comment that
/// precedes them. Best effort: returns the number of entries removed.
///
/// The caller must make sure no other known device still uses this IP.
pub fn remove_known_host(ip: &str) -> usize {
    let home_dir = match env::var("HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => return 0,
    };
    let known_hosts_path = home_dir.join(".ssh").join("known_hosts");
    let content = match fs::read_to_string(&known_hosts_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let bracketed = format!("[{ip}]:");
    let is_target = |line: &str| {
        line.split_whitespace()
            .next()
            .is_some_and(|patterns| {
                patterns
                    .split(',')
                    .any(|p| p == ip || p.starts_with(&bracketed))
            })
    };

    let mut kept: Vec<&str> = Vec::new();
    let mut removed = 0;
    for line in content.lines() {
        if is_target(line) {
            removed += 1;
            // Drop the regroup comment that introduced this entry.
            if kept
                .last()
                .is_some_and(|l| l.trim() == "# Added by regroup")
            {
                kept.pop();
            }
            continue;
        }
        kept.push(line);
    }
    if removed > 0 {
        let mut out = kept.join("\n");
        out.push('\n');
        let _ = fs::write(&known_hosts_path, out);
    }
    removed
}

/// Verify that the host key announced in a beacon is really served by the SSH
/// server at the announced ip:port, and pin the server's keys in
/// ~/.ssh/known_hosts for automatic verification on later connections.
///
/// This is a CRITICAL security check. When a device announces itself via UDP
/// beacon, it provides its SSH server host key. Before trusting the announced
/// identity, we verify that the actual SSH server on that IP serves the same
/// key. If the fingerprints don't match, it could indicate:
/// - a man-in-the-middle attack
/// - a forged beacon (someone claiming another machine's key)
/// - the device's SSH keys were regenerated
/// - network misconfiguration
///
/// All keys offered by the server are checked: servers may offer several
/// algorithms and ssh-keyscan returns them in arbitrary order.
///
/// Returns:
/// - HostKeyVerification::Success if verified and pinned
/// - HostKeyVerification::HostUnreachable if the host is offline
/// - HostKeyVerification::FingerprintMismatch if the fingerprints don't match (SECURITY ALERT!)
/// - HostKeyVerification::WriteError if we couldn't write to known_hosts
pub fn verify_and_add_to_known_hosts(
    ip: &str,
    port: u16,
    expected_fingerprint: &SshFingerprint,
) -> HostKeyVerification {
    // Fetch the keys the server actually offers at ip:port
    let keyscan_str = match keyscan(ip, port) {
        Ok(s) => s,
        Err(_) => return HostKeyVerification::HostUnreachable,
    };

    // The announced key must be among the keys the server offers.
    let mut first_served: Option<SshFingerprint> = None;
    for line in keyscan_str.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        // Extract key from ssh-keyscan output and normalize it
        let key = parts[1..].join(" ");
        let normalized_key = match normalize_public_key(&key) {
            Some(k) => k,
            None => continue,
        };
        if let Some(fp) = compute_fingerprint_from_public_key(&normalized_key) {
            if first_served.is_none() {
                first_served = Some(fp.clone());
            }
            if &fp == expected_fingerprint {
                // The server at ip:port really owns the announced key:
                // pin all its keys for MITM protection.
                return match append_known_hosts_entries(ip, port, &keyscan_str) {
                    Ok(()) => HostKeyVerification::Success,
                    Err(_) => HostKeyVerification::WriteError,
                };
            }
        }
    }

    HostKeyVerification::FingerprintMismatch {
        expected: expected_fingerprint.0.clone(),
        actual: first_served
            .map(|f| f.0)
            .unwrap_or_else(|| "unknown".to_string()),
    }
}
