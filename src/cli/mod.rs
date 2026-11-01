//! CLI module for regroup
//!
//! This module handles command-line interface functionality.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::{load_config, CONFIG_DIR, DEVICES_DIR, GIT_DIR, Machine};
use crate::discovery::host_name;
use crate::discovery::beacon::BeaconDevice;
use crate::config::device::save_device_file;
use crate::ssh::config::update_block;
use crate::ssh::public_key::{self};
use crate::git::{self, sync::git_add_and_commit_file};

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

    // 2. test config loading with sample data
    let sample_dir = PathBuf::from("/tmp/regroup-selftest");
    let _ = fs::remove_dir_all(&sample_dir);
    
    if let Err(e) = fs::create_dir_all(&sample_dir) {
        println!("temp dir creation — FAIL ({})", e);
        ok = false;
    } else {
        let devices_dir = sample_dir.join(DEVICES_DIR);
        let _ = fs::create_dir_all(&devices_dir);
        
        let device_file = devices_dir.join("test.json");
        let device_config = r#"{"name": "test", "ssh_public_key": "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAB", "user": "testuser", "port": 22}"#;
        if let Err(e) = fs::write(&device_file, device_config) {
            println!("device file creation — FAIL ({})", e);
            ok = false;
        } else {
            match crate::config::device::load_device_file(&device_file) {
                Ok(m) => {
                    let device_ok = m.name == "test" 
                        && m.ssh_public_key.as_str() == "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAB"
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
        "regroup — find machines by their SSH host key and update ~/.ssh/config.\nDevices announce themselves via signed UDP beacons; regroup listens for them\ncontinuously and prompts for each new device.\n\nConfiguration is stored in ~/.regroup-config/git/ (a git repository) where each\ndevice has its own JSON file in the devices/ directory. Git remotes can be\nconfigured for all other devices to enable decentralized sync (press 'r' in\ninteractive mode to pull from and push to them).\n\nUsage:\n  ./regroup                continuous interactive mode, update ~/.ssh/config\n  ./regroup test           dry run: nothing is modified\n  ./regroup -v             verbose\n  ./regroup selftest       built-in checks (tools, config parsing)\n  ./regroup setup-cd       install 'regroup-cd' shell function to change to config directory\n  ./regroup -h             this help\n\nInteractive commands:\n  y/N             - add or skip the current device\n  r               - refresh git (pull from remotes, push to remotes)\n  q               - quit the program\n\nConfig repository structure (~/.regroup-config/git/):\n  settings.json           (optional: udp_port, wait_secs)\n  devices/\n    <hostname>.json       (each device's config: name, ssh_public_key, user, port)\n\nExit codes:\n  0  normal exit\n  1  selftest failure\n  2  configuration or environment error"
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

// ============================================================
// Device management
// ============================================================

/// Check whether ~/.ssh/config content contains a "Host <name>" block.
/// Mirrors the block detection used by update_block, so that a name which
/// would overwrite an existing block is considered taken.
fn ssh_config_has_host(ssh_content: &str, name: &str) -> bool {
    for line in ssh_content.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() && words[0] == "Host" && words[1..].iter().any(|w| *w == name) {
            return true;
        }
    }
    false
}

/// Pick a device name that does not collide with an existing machine.
///
/// A name is taken when it is already used by:
/// - a machine in the regroup config (devices loaded from `devices/`),
/// - an existing `devices/<name>.json` file (even one that failed to load), or
/// - an existing "Host <name>" block in ~/.ssh/config, whether regroup-managed
///   or configured by the user for an unrelated host.
///
/// Returns `desired` if free, otherwise "name-2", "name-3", ... — the first
/// free candidate. This protects existing entries from being overwritten:
/// device files are per-name and update_block replaces Host blocks by name.
pub fn unique_device_name(desired: &str, ssh_config: &PathBuf, devices_dir: &PathBuf) -> String {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let known_machines = load_config().machines;
    let taken = |name: &str| {
        known_machines.iter().any(|m| m.name == name)
            || devices_dir.join(format!("{}.json", name)).exists()
            || ssh_config_has_host(&ssh_content, name)
    };
    if !taken(desired) {
        return desired.to_string();
    }
    let mut n = 2;
    while n < 1000 && taken(&format!("{}-{}", desired, n)) {
        n += 1;
    }
    format!("{}-{}", desired, n)
}

/// Add an accepted device to the configuration (device file, ~/.ssh/config,
/// known_hosts) and commit it to the config git repository.
///
/// If the device's name is already used by another machine — in the regroup
/// config, in an existing device file, or in an existing ~/.ssh/config Host
/// block — the device is renamed ("name-2", "name-3", ...) so existing
/// entries are never overwritten.
///
/// Returns the machine as actually saved (with its final name), or None if
/// the device file could not be written.
pub fn add_device_to_config(
    home: &str,
    ssh_config: &PathBuf,
    device: &BeaconDevice,
    new_count: &mut usize,
) -> Option<Machine> {
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    // Ensure devices directory exists
    if let Err(e) = fs::create_dir_all(&devices_dir) {
        warn(&format!("could not create devices directory: {}", e));
        return None;
    }
    
    // Rename the device if another machine already uses its name
    let name = unique_device_name(&device.name, ssh_config, &devices_dir);
    if name != device.name {
        println!(
            "  ! Name '{}' already used by another machine or an existing ~/.ssh/config entry — saved as '{}'",
            device.name, name
        );
    }
    
    let device_path = devices_dir.join(format!("{}.json", name));
    // The beacon explicitly carries the device's SSH port: mark it explicit so
    // ~/.ssh/config and the device file keep it (e.g. Termux port 8022).
    let machine = Machine {
        name: name.clone(),
        ssh_public_key: device.ssh_public_key.clone(),
        user: String::new(),
        port: device.port,
        port_explicit: true,
    };
    
    match save_device_file(&device_path, &machine) {
        Ok(_) => {
            // Update SSH config
            if let Err(e) = update_block(ssh_config, &name, &device.ip, &machine.user, machine.port, machine.port_explicit) {
                warn(&format!("could not update SSH config for {}: {}", name, e));
            } else {
                println!("  + Added {} -> {} (~/.ssh/config updated)", name, device.ip);
            }
            
            // Add to known_hosts using ssh-keyscan for MITM protection
            match public_key::add_to_known_hosts(&name, &device.ip, device.port) {
                true => {
                    println!("  + Added {} to ~/.ssh/known_hosts for SSH verification", name);
                }
                false => {
                    warn(&format!("could not add {} to ~/.ssh/known_hosts", name));
                }
            }
            
            // Commit to git so changes persist across runs
            if let Err(e) = git_add_and_commit_file(&config_dir, &device_path, &format!("Add device {}", name)) {
                warn(&format!("could not commit device config for {}: {}", name, e));
            }
            
            *new_count += 1;
            Some(machine)
        }
        Err(e) => {
            warn(&format!("could not save device config for {}: {}", name, e));
            None
        }
    }
}

// ============================================================
// Git operations
// ============================================================

/// Refresh git remotes: configure remotes for all known devices (except self),
/// then pull from all remotes and push to all remotes.
pub fn refresh_git(home: &str) {
    // The git repository lives in ~/.regroup-config/git (not the root directory)
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    
    if !git::is_git_repo(&config_dir) {
        warn(&format!("{} is not a git repository", config_dir.display()));
        return;
    }
    
    println!("\nRefreshing git...");
    
    // Configure remotes for all known machines before syncing: pull/push can
    // only reach devices that already have a remote.
    let remote_errors = configure_remotes(&config_dir);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }
    
    // Sync with remotes (pull then push)
    let sync_errors = git::sync_with_remotes(&config_dir);
    for error in &sync_errors {
        warn(&format!("[sync] {}", error));
    }
    
    // A pull may have brought device files for machines we did not know yet:
    // configure their remotes so the next refresh reaches them too.
    let remote_errors = configure_remotes(&config_dir);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }
    
    println!("Git refresh complete.");
}

/// Configure git remotes for all known machines except this device.
///
/// Self-detection uses both the hostname and the SSH public key: a stale
/// device file written under an old hostname (hostnames can change, e.g.
/// Termux) must not come back as a remote pointing at ourselves.
fn configure_remotes(config_dir: &PathBuf) -> Vec<String> {
    let conf = load_config();
    let local_name = host_name();
    let local_ssh_public_key = public_key::get_ssh_server_public_key();
    let machines: Vec<Machine> = conf
        .machines
        .into_iter()
        .filter(|m| {
            m.name != local_name
                && local_ssh_public_key.as_ref() != Some(&m.ssh_public_key)
        })
        .collect();
    git::refresh_git_remotes(config_dir, &machines, &local_name)
}
