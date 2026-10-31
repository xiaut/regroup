//! CLI module for regroup
//!
//! This module handles command-line interface functionality.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::{CONFIG_DIR, DEVICES_DIR};
use crate::discovery::{net_prefix, net_valid};

// ============================================================
// Error handling
// ============================================================

/// Terminate with an error message
pub fn die(msg: &str) -> ! {
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message
pub fn warn(msg: &str) {
    eprintln!("WARNING: {}", msg);
}

// ============================================================
// User interaction
// ============================================================

/// Ask user to resolve a conflict
/// Returns: true = keep local, false = use remote, None = abort
pub fn ask_conflict_resolution(device_name: &str, local_content: &str, remote_content: &str) -> Option<bool> {
    use std::io::Write;
    
    println!("\nCONFLICT: Device config for '{}' differs from remote", device_name);
    println!("Local version:");
    println!("{}", local_content);
    println!("\nRemote version:");
    println!("{}", remote_content);
    println!("\nOptions:");
    println!("  1. Keep local version");
    println!("  2. Use remote version");
    println!("  3. Abort");
    print!("\nChoice [1/2/3]: ");
    std::io::stdout().flush().ok();
    
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        match input.trim() {
            "1" => Some(true),
            "2" => Some(false),
            "3" => None,
            _ => Some(true), // Default to keep local
        }
    } else {
        Some(true)
    }
}



// ============================================================
// Selftest
// ============================================================

/// Run built-in selftest
pub fn run_selftest() -> ! {
    let mut ok = true;

    // 1. external tools
    let mut tools_ok = true;
    for tool in ["ssh-keyscan", "ssh-keygen"] {
        if Command::new(tool).output().is_err() {
            tools_ok = false;
        }
    }
    // Check for git
    if Command::new("git").output().is_err() {
        tools_ok = false;
    }
    
    println!(
        "ssh-keyscan/ssh-keygen/git present — {}",
        if tools_ok { "OK" } else { "FAIL" }
    );
    ok &= tools_ok;

    // 2. subnet helpers
    let net_ok = net_valid(&net_prefix("192.168.43.0/24")) == Some("192.168.43".to_string())
        && net_valid(&net_prefix("192.168.43")) == Some("192.168.43".to_string())
        && net_valid("192.168.43") == Some("192.168.43".to_string())
        && net_valid("192.168.256") == None
        && net_valid("192.168") == None
        && net_valid("abc.def.ghi") == None;
    println!("subnet helpers — {}", if net_ok { "OK" } else { "FAIL" });
    ok &= net_ok;

    // 3. test config loading with sample data
    let sample_dir = PathBuf::from("/tmp/regroup-selftest");
    let _ = fs::remove_dir_all(&sample_dir);
    
    if let Err(e) = fs::create_dir_all(&sample_dir) {
        println!("temp dir creation — FAIL ({})", e);
        ok = false;
    } else {
        let devices_dir = sample_dir.join(DEVICES_DIR);
        let _ = fs::create_dir_all(&devices_dir);
        
        let device_file = devices_dir.join("test.json");
        let device_config = r#"{"name": "test", "fingerprint": "SHA256:abc", "user": "testuser", "port": 22}"#;
        if let Err(e) = fs::write(&device_file, device_config) {
            println!("device file creation — FAIL ({})", e);
            ok = false;
        } else {
            match crate::config::device::load_device_file(&device_file) {
                Ok(m) => {
                    let device_ok = m.name == "test" 
                        && m.fingerprint == "SHA256:abc"
                        && m.user == "testuser"
                        && m.port == 22
                        && m.port_explicit;
                    println!("device config loading — {}", if device_ok { "OK" } else { "FAIL" });
                    ok &= device_ok;
                }
                Err(e) => {
                    println!("device config loading — FAIL ({})", e);
                    ok = false;
                }
            }
        }
        
        let _ = fs::remove_dir_all(&sample_dir);
    }

    println!("selftest: {}", if ok { "OK" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}

// ============================================================
// Usage
// ============================================================

/// Print usage information
pub fn usage() {
    println!(
        "regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config. Every run announces this machine over UDP broadcast while
listening for the other machines' announcements during the same window.\n
Configuration is stored in ~/.regroup-config/ git repository where each
device has its own JSON file in the devices/ directory. Git remotes are
automatically configured for all other devices (using hostnames, not IPs).\n\nDiscovery and sync run in a loop until no new devices are found. Each device
owns its own config file and can only overwrite it. If the local device config
differs from the remote, the user is asked to resolve the conflict.\n
Usage:
  ./regroup                announce + listen + sync loop, update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -r             refresh: pull from git remotes before sync loop
  ./regroup 192.168.43     also scan that /24 once
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup setup-cd       install 'regroup-cd' shell function to change to config directory
  ./regroup -h             this help\n
Config repository structure (~/.regroup-config/):
  settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
  devices/
    <hostname>.json       (each device's config: name, fingerprint, user, port)\n
Git remotes (automatically managed using hostnames):
  user@hostname:~/.regroup-config  (for each other device)\n
Sync behavior:
  - Loop until no new devices are discovered
  - git pull --all on each iteration (except first)
  - git push --all after updating SSH config
  - One-way SSH connections handled gracefully
  - User prompted to resolve own config conflicts\n
Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
  3  user declined conflict resolution"
    );
}

// ============================================================
// Setup CD command
// ============================================================

/// Install the regroup-cd shell function in the user's shell config
pub fn setup_cd() -> ! {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };

    let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
    let function_def = format!("regroup-cd() {{ cd {}; }}", config_dir.to_string_lossy());

    // Detect shell from SHELL environment variable
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let config_file = if shell.contains("zsh") {
        PathBuf::from(&home).join(".zshrc")
    } else {
        PathBuf::from(&home).join(".bashrc")
    };

    // Read current content
    let current_content = if config_file.exists() {
        match fs::read_to_string(&config_file) {
            Ok(c) => c,
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    // Prepare new content - remove any existing definition
    let new_content: String = current_content
        .lines()
        .filter(|line| !line.trim().starts_with("regroup-cd()"))
        .collect::<Vec<_>>()
        .join("\n");

    // Find the line with "case $- in" (non-interactive shell check)
    // We want to insert BEFORE this block so the function is available even when
    // the shell returns early for non-interactive sessions
    let case_line_index = new_content.lines().position(|line| line.contains("case $- in"));

    // Add the function definition before the case block, or at the end
    let final_content = if let Some(idx) = case_line_index {
        let mut lines: Vec<&str> = new_content.lines().collect();
        // Insert before the case block, but after any leading comments/blank lines
        // Actually, just insert right before the case line
        lines.insert(idx, &function_def);
        lines.join("\n")
    } else if !new_content.is_empty() && !new_content.ends_with('\n') {
        format!("{}\n\n{}", new_content, function_def)
    } else if new_content.is_empty() {
        function_def
    } else {
        format!("{}{}", new_content, function_def)
    };

    // Write to config file
    match fs::write(&config_file, final_content) {
        Ok(_) => {
            println!(
                "✓ 'regroup-cd' function installed in ~/{}",
                config_file.file_name().unwrap_or_default().to_string_lossy()
            );
            println!("Run 'source ~/{}' or open a new terminal to start using it.",
                config_file.file_name().unwrap_or_default().to_string_lossy());
            println!("Use: regroup-cd");
        }
        Err(e) => die(&format!(
            "Cannot write to ~/{}: {}",
            config_file.file_name().unwrap_or_default().to_string_lossy(),
            e
        )),
    }

    std::process::exit(0);
}
