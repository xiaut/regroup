//! SSH public key detection module
//!
//! This module provides utilities for detecting the local machine's SSH public key.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Get the local machine's public SSH key fingerprint
/// Tries multiple methods in order of preference:
/// 1. Read from ~/.ssh/id_*.pub files (user keys)
/// 2. Read from /etc/ssh/ssh_host_*_key.pub files (system keys)
/// 3. Use ssh-keyscan on localhost (if SSH server is running)
/// 4. Return None if no key can be found
pub fn get_local_public_key_fingerprint() -> Option<String> {
    // First, try to get the fingerprint from local key files
    if let Some(fingerprint) = get_fingerprint_from_key_files() {
        return Some(fingerprint);
    }
    
    // Second, try ssh-keyscan on localhost
    if let Some(fingerprint) = get_fingerprint_from_keyscan() {
        return Some(fingerprint);
    }
    
    None
}

/// Get SSH host key fingerprint from local key files
/// Checks ~/.ssh/id_*.pub and /etc/ssh/ssh_host_*_key.pub
fn get_fingerprint_from_key_files() -> Option<String> {
    // List of possible key file patterns
    let key_patterns = [
        "id_rsa.pub",
        "id_ecdsa.pub",
        "id_ed25519.pub",
        "id_dsa.pub",
    ];
    
    // 1. Check user's SSH keys in ~/.ssh/
    if let Ok(home) = env::var("HOME") {
        let ssh_dir = PathBuf::from(home).join(".ssh");
        for pattern in &key_patterns {
            let key_path = ssh_dir.join(pattern);
            if let Some(fingerprint) = read_key_fingerprint(&key_path) {
                return Some(fingerprint);
            }
        }
    }
    
    // 2. Check system SSH host keys in /etc/ssh/
    let etc_ssh_dir = PathBuf::from("/etc/ssh");
    if etc_ssh_dir.exists() {
        let host_key_patterns = [
            "ssh_host_rsa_key.pub",
            "ssh_host_ecdsa_key.pub",
            "ssh_host_ed25519_key.pub",
            "ssh_host_dsa_key.pub",
        ];
        
        for pattern in &host_key_patterns {
            let key_path = etc_ssh_dir.join(pattern);
            if let Some(fingerprint) = read_key_fingerprint(&key_path) {
                return Some(fingerprint);
            }
        }
    }
    
    // 3. Check data directory for Termux (Termux stores keys in $PREFIX/etc/ssh/)
    if let Ok(prefix) = env::var("PREFIX") {
        let termux_ssh_dir = PathBuf::from(prefix).join("etc/ssh");
        if termux_ssh_dir.exists() {
            let host_key_patterns = [
                "ssh_host_rsa_key.pub",
                "ssh_host_ecdsa_key.pub",
                "ssh_host_ed25519_key.pub",
                "ssh_host_dsa_key.pub",
            ];
            
            for pattern in &host_key_patterns {
                let key_path = termux_ssh_dir.join(pattern);
                if let Some(fingerprint) = read_key_fingerprint(&key_path) {
                    return Some(fingerprint);
                }
            }
        }
    }
    
    None
}

/// Read a public key file and compute its fingerprint
fn read_key_fingerprint(key_path: &PathBuf) -> Option<String> {
    if !key_path.exists() {
        return None;
    }
    
    // Read the public key file (we just check existence, ssh-keygen reads it itself)
    if fs::read_to_string(key_path).is_err() {
        return None;
    }
    
    // Use ssh-keygen to compute the fingerprint
    use std::process::{Command, Stdio};
    
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")
        .arg(key_path)
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
    let fields: Vec<&str> = lines[0].split_whitespace().collect();
    for field in &fields {
        if field.starts_with("SHA256:") || field.starts_with("MD5:") {
            return Some(field.to_string());
        }
    }
    
    None
}

/// Get SSH host key fingerprint by running ssh-keyscan on localhost
fn get_fingerprint_from_keyscan() -> Option<String> {
    use std::process::{Command, Stdio};
    use std::io::Write;
    
    // Try to run ssh-keyscan on localhost
    let scan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    
    let raw = match scan {
        Ok(o) if !o.stdout.is_empty() => o.stdout,
        _ => return None,
    };
    
    // Parse the output with ssh-keygen to get the fingerprint
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
    
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&raw);
    }
    
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return None,
    };
    
    // Parse the first SHA256 fingerprint from the output
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 && fields[1].starts_with("SHA256:") {
            return Some(fields[1].to_string());
        }
    }
    
    None
}
