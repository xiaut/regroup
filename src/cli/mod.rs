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
        let device_config = r#"{"name": "test", "ssh_public_key": "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAB", "signing_public_key": "MCowBQYDK2VwAyEAGXNlbGZ0ZXN0a2V5eXl5eXl5eXl5eXl5eXl5eXl5eXl5eXl5eXk=", "user": "testuser", "port": 22}"#;
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

    // 3. host key fingerprint verification helper
    // Generate a throwaway key, then check that the fingerprint computed by
    // compute_fingerprint_from_public_key matches ssh-keygen -lf's output.
    {
        let key_path = PathBuf::from("/tmp/regroup-selftest-key");
        let pub_path = PathBuf::from("/tmp/regroup-selftest-key.pub");
        let _ = fs::remove_file(&key_path);
        let _ = fs::remove_file(&pub_path);

        let generated = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-q", "-f"])
            .arg(key_path.to_string_lossy().as_ref())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let fingerprint_ok = if !generated {
            false
        } else if let Ok(pub_content) = fs::read_to_string(&pub_path) {
            // Announced-style key: "key_type key_base64" without comment
            let announced = pub_content
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ");
            let computed = crate::ssh::public_key::compute_fingerprint_from_public_key(&announced);
            let reference = Command::new("ssh-keygen")
                .arg("-lf")
                .arg(pub_path.to_string_lossy().as_ref())
                .output()
                .ok()
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .find(|f| f.starts_with("SHA256:"))
                        .map(|f| f.to_string())
                });
            match (computed, reference) {
                (Some(c), Some(r)) => c.0 == r,
                _ => false,
            }
        } else {
            false
        };
        println!("host key fingerprint — {}", if fingerprint_ok { "OK" } else { "FAIL" });
        ok &= fingerprint_ok;

        let _ = fs::remove_file(&key_path);
        let _ = fs::remove_file(&pub_path);
    }

    // 4. signed beacon verification (roundtrip, tampering, malformed input)
    {
        use crate::signing;
        use crate::ssh::keys::SshPublicKey;

        let mut beacons_ok = true;
        if let Ok((signing_key, _)) = signing::generate_keypair() {
            let ssh_key = SshPublicKey::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAISelftestKeyNotReal");
            match signing::sign_beacon("selftest", &ssh_key, "10.1.2.3", 22, "0", &signing_key) {
                Ok(beacon) => {
                    // roundtrip
                    match signing::verify_beacon(&beacon, None) {
                        Ok((name, key, ip, port)) => {
                            beacons_ok &= name == "selftest"
                                && key == ssh_key
                                && ip == "10.1.2.3"
                                && port == 22;
                        }
                        Err(_) => beacons_ok = false,
                    }
                    // a tampered field must break the signature
                    let tampered = beacon.replace("10.1.2.3", "10.9.9.9");
                    beacons_ok &= signing::verify_beacon(&tampered, None).is_err();
                }
                Err(_) => beacons_ok = false,
            }
            // a malformed signing key must be rejected, never panic
            // (regression: this used to crash the process)
            let malformed = format!("selftest\tAAAA\tssh-ed25519 AAAA\t10.1.2.3\t22\t0\tAAAA");
            beacons_ok &= signing::verify_beacon(&malformed, None).is_err();
        } else {
            beacons_ok = false;
        }
        println!("signed beacon verification — {}", if beacons_ok { "OK" } else { "FAIL" });
        ok &= beacons_ok;
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

/// Path of the regroup config devices directory for the given home.
fn devices_dir_for(home: &str) -> PathBuf {
    PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR).join(DEVICES_DIR)
}

/// Check whether a device name is already used by another machine.
///
/// A name is taken when it is already used by:
/// - a machine in the regroup config (devices loaded from `devices/`),
/// - an existing `devices/<name>.json` file (even one that failed to load), or
/// - an existing "Host <name>" block in ~/.ssh/config, whether regroup-managed
///   or configured by the user for an unrelated host.
pub fn device_name_taken(name: &str, home: &str, ssh_config: &PathBuf) -> bool {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    name_taken(name, &ssh_content, &devices_dir_for(home), &load_config().machines)
}

fn name_taken(name: &str, ssh_content: &str, devices_dir: &PathBuf, known_machines: &[Machine]) -> bool {
    known_machines.iter().any(|m| m.name == name)
        || devices_dir.join(format!("{}.json", name)).exists()
        || ssh_config_has_host(ssh_content, name)
}

/// Suggest the first free name for a device: `desired` if free, otherwise
/// "name-2", "name-3", ... Used as the default answer when asking the user
/// to pick a new name for a device whose name is already taken.
pub fn unique_device_name(desired: &str, home: &str, ssh_config: &PathBuf) -> String {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let known_machines = load_config().machines;
    let devices_dir = devices_dir_for(home);
    if !name_taken(desired, &ssh_content, &devices_dir, &known_machines) {
        return desired.to_string();
    }
    let mut n = 2;
    while n < 1000 && name_taken(&format!("{}-{}", desired, n), &ssh_content, &devices_dir, &known_machines) {
        n += 1;
    }
    format!("{}-{}", desired, n)
}

/// Add an accepted device to the configuration (device file, ~/.ssh/config,
/// known_hosts) and commit it to the config git repository, under the given
/// name.
///
/// The caller is responsible for the name: interactive mode asks the user to
/// pick a collision-free name first (see `device_name_taken`); an unavailable
/// name would overwrite an existing device file or ~/.ssh/config block.
///
/// Returns the machine as actually saved, or None if the device file could
/// not be written.
pub fn add_device_to_config(
    home: &str,
    ssh_config: &PathBuf,
    device: &BeaconDevice,
    name: &str,
    new_count: &mut usize,
) -> Option<Machine> {
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    // Ensure devices directory exists
    if let Err(e) = fs::create_dir_all(&devices_dir) {
        warn(&format!("could not create devices directory: {}", e));
        return None;
    }
    
    let device_path = devices_dir.join(format!("{}.json", name));
    // The beacon explicitly carries the device's SSH port: mark it explicit so
    // ~/.ssh/config and the device file keep it (e.g. Termux port 8022).
    // Pin the device's Ed25519 signing key: future beacons must be signed
    // with this same key or the user is asked to confirm the change.
    let machine = Machine {
        name: name.to_string(),
        ssh_public_key: device.ssh_public_key.clone(),
        signing_public_key: device.signing_public_key.clone(),
        user: String::new(),
        port: device.port,
        port_explicit: true,
    };
    
    // SECURITY: verify that the host key announced in the beacon is really
    // served by the SSH server at the announced ip:port, BEFORE writing
    // anything. A mismatch means a forged beacon or a MITM: refuse the device.
    match public_key::compute_fingerprint_from_public_key(device.ssh_public_key.as_str()) {
        Some(expected) => match public_key::verify_and_add_to_known_hosts(&device.ip, device.port, &expected) {
            public_key::HostKeyVerification::Success => {
                println!("  + Added {} to ~/.ssh/known_hosts (host key verified)", name);
            }
            public_key::HostKeyVerification::HostUnreachable => {
                // Host unreachable: we cannot verify. Do not pin anything;
                // ssh will ask the user on first connection.
                warn(&format!(
                    "{} unreachable at {} — host key not verified, ssh will ask on first connection",
                    name, device.ip
                ));
            }
            public_key::HostKeyVerification::WriteError => {
                warn(&format!("could not write {} to ~/.ssh/known_hosts", name));
            }
            public_key::HostKeyVerification::FingerprintMismatch { expected, actual } => {
                eprintln!();
                eprintln!("[!] SECURITY: {} announces key {} at {}", name, &device.ssh_public_key.as_str()[..16.min(device.ssh_public_key.as_str().len())], device.ip);
                eprintln!("    but the server there serves a DIFFERENT host key — possible spoofing or MITM.");
                eprintln!("    expected fingerprint: {}", expected);
                eprintln!("    actual fingerprint:   {}", actual);
                eprintln!("    Device NOT added.");
                return None;
            }
        },
        None => {
            eprintln!("[!] SECURITY: {} announces a host key that is not a valid OpenSSH public key — device NOT added.", name);
            return None;
        }
    }
    
    match save_device_file(&device_path, &machine) {
        Ok(_) => {
            // Update SSH config
            if let Err(e) = update_block(ssh_config, name, &device.ip, &machine.user, machine.port, machine.port_explicit) {
                warn(&format!("could not update SSH config for {}: {}", name, e));
            } else {
                println!("  + Added {} -> {} (~/.ssh/config updated)", name, device.ip);
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

/// Persist a device's signing public key in its device file and commit the
/// change. Used when the user accepts a new signing key (machine reinstall).
pub fn trust_new_signing_key(home: &str, device: &BeaconDevice) {
    let conf = load_config();
    let machine = conf
        .machines
        .iter()
        .find(|m| m.ssh_public_key == device.ssh_public_key)
        .cloned();
    let mut machine = match machine {
        Some(m) => m,
        None => return,
    };
    machine.signing_public_key = device.signing_public_key.clone();
    
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    let device_path = config_dir.join(DEVICES_DIR).join(format!("{}.json", machine.name));
    if let Err(e) = save_device_file(&device_path, &machine) {
        warn(&format!("could not update signing key for {}: {}", machine.name, e));
        return;
    }
    if let Err(e) = git_add_and_commit_file(&config_dir, &device_path, &format!("Update signing key for {}", machine.name)) {
        warn(&format!("could not commit signing key update for {}: {}", machine.name, e));
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
