//! Discovery and sync loop module
//!
//! This module contains the main discovery and synchronization loop.
//!
//! Critical design note: SSH config (~/.ssh/config) MUST be updated BEFORE
//! any git operations, because git uses SSH to connect to remotes and
//! requires the SSH config to know how to reach each machine.
//!
//! Order of operations:
//! 1. Discovery (UDP beacons only - no TCP scanning) - devices announce themselves
//!    with name, public_key, and IP. Each device detects its own public key locally.
//! 2. Auto-add new devices - devices with unknown public keys are automatically
//!    added to local configuration (config is backup, not source of truth).
//! 3. Update SSH config - so git knows how to connect to each machine
//! 4. Refresh git remotes - ensure git knows about all machines
//! 5. Git sync (pull/push) - exchange configuration files

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;

use super::{announce_beacon, get_local_interfaces, listen_beacons, BeaconDevice};
use crate::discovery::DiscoveryResult;
use crate::cli::{ask_conflict_resolution, die, warn};
use crate::ssh::public_key::get_ssh_server_public_key;
use crate::signing;
use crate::config::{Conf, CONFIG_DIR, DEVICES_DIR, GLOBAL_CONFIG_FILE, MAX_SYNC_ITERATIONS, load_config, read_device_file_content};
use crate::git::{git_add_and_commit_file, get_modified_config_files, git_push_all, has_unstaged_changes, is_git_repo, refresh_git_remotes, sync_with_remotes};
use crate::ssh::{flush_ssh_config, update_block, verify_ssh_config_complete};

/// Log the current phase of the sync process
fn log_phase(verbose: bool, iteration: usize, phase: u8, total_phases: u8, message: &str) {
    if verbose || iteration == 1 {
        println!("[{}/{}] {}", phase, total_phases, message);
    }
}

/// Run discovery and sync in a loop until no new devices are found
pub fn discovery_sync_loop(
    conf: &Conf,
    ssh_config: &Path,
    local_name: &str,
    local_ip: &Option<String>,
    dry_run: bool,
    verbose: bool,
    initial_refresh: bool,
    _net_arg: &Option<String>,
) -> DiscoveryResult {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    // Start with the initial configuration
    let mut current_conf = conf.clone();
    
    // Check if there are unstaged changes to config files and load them
    if has_unstaged_changes(&config_dir) {
        if verbose {
            println!("Detected unstaged config changes - reloading configuration");
        }
        // Reload config to pick up local modifications
        current_conf = load_config();
        if verbose {
            println!("Configuration reloaded with local changes");
        }
    }
    
    // Get initial list of device files
    let mut previous_device_files: HashSet<String> = HashSet::new();
    if devices_dir.exists() {
        if let Ok(entries) = fs::read_dir(&devices_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if entry.path().is_file() && entry.path().extension().map_or(false, |ext| ext == "json") {
                        if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                            previous_device_files.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }
    
    let mut found = 0;
    let mut updated = 0;
    let mut missing = 0;
    let mut new_count = 0;
    let mut iteration = 0;
    let mut new_devices_found = true;
    let mut has_local_changes = false;
    
    // Check if there are local config changes (unstaged) at the start
    // This includes settings.json modifications and new device files
    if is_git_repo(&config_dir) && has_unstaged_changes(&config_dir) {
        has_local_changes = true;
        if verbose {
            let modified = get_modified_config_files(&config_dir);
            println!("Local config changes detected: {:?}", modified);
        }
    }
    
    // Check if this device is alone in the config (only itself configured)
    let is_alone = current_conf.machines.len() == 1 && current_conf.machines[0].name == local_name;
    
    // Save the current state of our device file before any operations
    let local_device_path = config_dir.join(DEVICES_DIR).join(format!("{}.json", local_name));
    let our_initial_config = if local_device_path.exists() {
        read_device_file_content(&local_device_path)
    } else {
        String::new()
    };
    
    // Track settings.json for changes
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    let _our_initial_settings = if settings_path.exists() {
        read_device_file_content(&settings_path)
    } else {
        String::new()
    };
    
    while (new_devices_found || (is_alone && iteration < 2)) && iteration < MAX_SYNC_ITERATIONS {
        iteration += 1;
        new_devices_found = false;
        
        // Check for new unstaged config changes at the start of each iteration
        // This allows immediate use of new device configs without requiring a commit
        if is_git_repo(&config_dir) && has_unstaged_changes(&config_dir) {
            let modified_files = get_modified_config_files(&config_dir);
            let settings_modified = modified_files.iter().any(|p| p == Path::new(GLOBAL_CONFIG_FILE));
            let devices_modified = modified_files.iter().any(|p| p.to_string_lossy().starts_with(&format!("{}/", DEVICES_DIR)));
            
            if settings_modified || devices_modified {
                if verbose {
                    println!("  . Detected config changes: {:?}", modified_files);
                }
                // Reload configuration to pick up the changes
                current_conf = load_config();
                has_local_changes = true;
                
                // Update is_alone flag with new config
                if current_conf.machines.len() == 1 && current_conf.machines[0].name == local_name {
                    // Still alone, but we'll continue with the loop
                }
            }
        }
        
        // ============================================================
        // PHASE 1: Discovery
        // ============================================================
        // Find machines via UDP beacons only (no TCP scanning)
        // ============================================================
        log_phase(verbose, iteration, 1, 4, "Discovery");
        
        // Get local SSH public key and signing key by self-detection
        let local_ssh_port = current_conf.machines
            .iter()
            .find(|m| m.name == local_name)
            .map(|m| m.port)
            .unwrap_or(22);
        let local_ssh_public_key = get_ssh_server_public_key();
        
        let local_interfaces = get_local_interfaces();
        let local_ip = local_interfaces.first().map(|iface| iface.ip.clone()).unwrap_or_default();
        
        // Initialize announcer thread (may be None if we can't announce)
        let announcer: Option<thread::JoinHandle<()>> = if local_ssh_public_key.is_none() {
            if iteration == 1 {
                warn("no SSH server public key detected — this device will not announce itself");
            }
            None
        } else {
            // Load or generate signing key
            match signing::initialize_signing_keys() {
                Ok((signing_key, _)) => {
                    if !local_interfaces.is_empty() {
                        let port = current_conf.udp_port;
                        let secs = current_conf.wait_secs;
                        let name = local_name.to_string();
                        let ssh_public_key = local_ssh_public_key.as_ref().unwrap().clone();
                        let ip = local_ip.clone();
                        let port_ssh = local_ssh_port;
                        let interfaces = local_interfaces.clone();
                        let signing_key_clone = signing_key.clone();
                        Some(thread::spawn(move || {
                            announce_beacon(port, secs, &name, &ssh_public_key, &ip, port_ssh, &signing_key_clone, &interfaces)
                        }))
                    } else {
                        if iteration == 1 {
                            warn("no local interfaces found — announcing disabled (listening continues)");
                        }
                        None
                    }
                }
                Err(e) => {
                    if iteration == 1 {
                        warn(&format!("cannot initialize signing keys: {}", e));
                    }
                    None
                }
            }
        };
        
        let beacons = listen_beacons(current_conf.udp_port, current_conf.wait_secs);
        
        // Join the announcer thread if it exists
        if let Some(h) = announcer {
            let _ = h.join();
        }
        
        // If we couldn't get SSH public key or signing keys, continue with listening only
        let local_ssh_public_key = match local_ssh_public_key {
            Some(key) => key,
            None => String::new(),
        };
        
        if verbose {
            println!("  . {} beacon(s) received", beacons.len());
            for b in &beacons {
                let key_preview = if b.ssh_public_key.len() > 8 {
                    format!("{}...", &b.ssh_public_key[..8])
                } else {
                    b.ssh_public_key.clone()
                };
                println!("  . beacon: {} ({}, key: {})", b.name, b.ip, key_preview);
            }
        }
        
        // Build set of known SSH public keys from config
        let known_keys: HashSet<String> = current_conf.machines.iter().map(|m| m.ssh_public_key.clone()).collect();
        
        // Find new devices (beacons with unknown SSH public keys)
        // Note: BeaconDevice uses 'ssh_public_key' field for SSH public key
        let new_devices: Vec<&BeaconDevice> = beacons.iter()
            .filter(|b| !known_keys.contains(&b.ssh_public_key) && !b.ssh_public_key.is_empty() && b.ssh_public_key != local_ssh_public_key)
            .collect();
        
        // Ask user to confirm new devices (identified by unknown public key)
        if !new_devices.is_empty() && iteration == 1 {
            println!("== New devices detected ==");
            
            // Ask user for each device individually (skip in dry run)
            if !dry_run {
                for device in &new_devices {
                    // Display device info including SSH public key for verification
                    let key_display = if device.ssh_public_key.len() > 20 {
                        format!("{}...", &device.ssh_public_key[..17])
                    } else {
                        device.ssh_public_key.clone()
                    };
                    print!("Add {} ({}) with key {}? [y/N]: ", device.name, device.ip, key_display);
                    io::stdout().flush().ok();
                    
                    let mut answer = String::new();
                    if io::stdin().read_line(&mut answer).is_ok() {
                        if answer.trim().to_lowercase() == "y" || answer.trim().to_lowercase() == "yes" {
                            use crate::config::device::save_device_file;
                            use crate::config::Machine;
                            
                            // Ensure devices directory exists
                            if let Err(e) = fs::create_dir_all(&devices_dir) {
                                warn(&format!("could not create devices directory: {}", e));
                                continue;
                            }
                            let device_path = devices_dir.join(format!("{}.json", device.name));
                            let machine = Machine {
                                name: device.name.clone(),
                                ssh_public_key: device.ssh_public_key.clone(),
                                user: String::new(),
                                port: device.port,
                                port_explicit: false,
                            };
                            
                            if let Err(e) = save_device_file(&device_path, &machine) {
                                warn(&format!("could not save device config for {}: {}", device.name, e));
                            } else {
                                println!("  + Added {} to config", device.name);
                                current_conf.machines.push(machine);
                                has_local_changes = true;
                                new_count += 1;
                            }
                        }
                    }
                }
            } else {
                println!("  (dry run - not adding devices)");
            }
        }
        
        if beacons.is_empty() && iteration == 1 {
            warn("no beacons received — run regroup on other machines during the window");
        }
        
        // Build a map from SSH public key to beacon for easy lookup
        let beacon_map: std::collections::HashMap<String, &BeaconDevice> = beacons.iter()
            .map(|b| (b.ssh_public_key.clone(), b))
            .collect();

        // ============================================================
        // PHASE 2: Update SSH config
        // ============================================================
        // Update ~/.ssh/config with discovered IPs so that git can connect
        // This MUST happen BEFORE any git operations (pull/push)
        // ============================================================
        log_phase(verbose, iteration, 2, 4, "Updating SSH config (~/.ssh/config)");
        
        let mut ssh_updated = 0;
        let mut ssh_failed: Vec<String> = Vec::new();
        
        // --- machines ---
        if iteration == 1 {
            println!("== Machines ==");
        }
        let mut iteration_found = 0;
        let mut iteration_missing = 0;
        
        for m in &current_conf.machines {
            // Find matching beacon by SSH public key
            if let Some(beacon) = beacon_map.get(&m.ssh_public_key) {
                // Check if this is the local machine
                if Some(local_ip.as_str()) == Some(beacon.ip.as_str()) {
                    if iteration == 1 {
                        println!(
                            "  o {}: this is the local machine (this host), config unchanged",
                            m.name
                        );
                    }
                    // Don't count local machine in the found total
                    continue;
                }
                
                // Found a matching beacon
                if dry_run {
                    if iteration == 1 {
                        println!("  ~ {} -> {} (test mode, config not modified)", m.name, beacon.ip);
                    }
                    iteration_found += 1;
                } else {
                    match update_block(&ssh_config, &m.name, &beacon.ip, &m.user, m.port, m.port_explicit)
                    {
                        Ok(()) => {
                            if iteration == 1 {
                                println!("  ~ {} -> {}  (~/.ssh/config updated)", m.name, beacon.ip);
                            }
                            iteration_found += 1;
                            ssh_updated += 1;
                        }
                        Err(e) => {
                            warn(&format!("could not write ~/.ssh/config for {}: {}", m.name, e));
                            iteration_missing += 1;
                            ssh_failed.push(m.name.clone());
                            continue;
                        }
                    }
                }
            } else {
                // No matching beacon found
                if iteration == 1 {
                    println!("  x {}: not found (no beacon with matching public key)", m.name);
                }
                iteration_missing += 1;
            }
        }
        
        // Force SSH config to be written to disk before git operations
        if !dry_run && ssh_updated > 0 {
            if let Err(e) = flush_ssh_config(ssh_config) {
                warn(&format!("Failed to flush SSH config to disk: {}", e));
            }
        }
        
        // Check if SSH config is complete for all machines
        if !dry_run && !ssh_failed.is_empty() {
            warn(&format!(
                "SSH config update failed for: {}. Git sync may not work for these machines.",
                ssh_failed.join(", ")
            ));
        }
        
        updated = iteration_found;
        missing = iteration_missing;
        
        // ============================================================
        // PHASE 3: Git remotes configuration
        // ============================================================
        // Configure git remotes for all machines
        // Requires SSH config to be up to date (Phase 2)
        // ============================================================
        log_phase(verbose, iteration, 3, 4, if dry_run { "Configuring git remotes - skipped (dry run)" } else { "Configuring git remotes" });
        
        if !dry_run {
            // Verify SSH config is complete before git operations
            let ssh_missing = verify_ssh_config_complete(ssh_config, &current_conf.machines);
            if !ssh_missing.is_empty() {
                warn(&format!(
                    "SSH config missing entries for: {}. Git operations may fail for these machines.",
                    ssh_missing.join(", ")
                ));
            }
            
            let errors = refresh_git_remotes(&config_dir, &current_conf.machines, local_name);
            for error in &errors {
                warn(&format!("git remote refresh: {}", error));
            }
            
            // Sync with remotes after discovery and SSH config update
            // On first iteration, only sync if initial_refresh flag is set
            log_phase(verbose, iteration, 4, 4, 
                if dry_run {
                    "Syncing with git remotes (pull/push) - skipped (dry run)"
                } else if iteration > 1 || initial_refresh {
                    "Syncing with git remotes (pull/push)"
                } else {
                    "Syncing with git remotes (pull/push) - skipped"
                }
            );
            
            if iteration > 1 || initial_refresh {
                let sync_errors = sync_with_remotes(&config_dir, has_local_changes);
                for error in &sync_errors {
                    // Always warn about sync errors, even in non-verbose mode
                    // The error message already includes SSH auth setup hint
                    warn(&format!("[sync] {}", error));
                    if verbose {
                        println!("  [sync] {}", error);
                    }
                }
                
                // After pulling, check if our own device config was modified
                // and if it conflicts with our initial state
                if !sync_errors.is_empty() || iteration > 1 {
                    if local_device_path.exists() {
                        // Check if the current version differs from our initial state
                        let current_content = read_device_file_content(&local_device_path);
                        
                        // Compare with initial state
                        if current_content.trim() != our_initial_config.trim() {
                            // Config was modified by the pull - ask user to resolve
                            // The current_content is what was pulled from remote
                            // our_initial_config is what we had before
                            
                            if let Some(keep_local) = ask_conflict_resolution(
                                local_name, 
                                &our_initial_config,  // Local version (before pull)
                                &current_content       // Remote version (after pull)
                            ) {
                                if keep_local {
                                    // User wants to keep local version - revert to initial
                                    if let Err(e) = fs::write(&local_device_path, &our_initial_config) {
                                        warn(&format!("failed to revert device config: {}", e));
                                    } else {
                                        // Commit the reverted device file separately
                                        let message = format!("Revert device: {} to local version", local_name);
                                        let _ = git_add_and_commit_file(&config_dir, &local_device_path, &message);
                                        // Mark that we have local changes to push
                                        has_local_changes = true;
                                    }
                                } else {
                                    // User wants remote version - it's already in place from the pull
                                    // Commit it separately
                                    let message = format!("Update device: {} from remote", local_name);
                                    let _ = git_add_and_commit_file(&config_dir, &local_device_path, &message);
                                    // Mark that we have local changes to push
                                    has_local_changes = true;
                                }
                            } else {
                                // User aborted
                                process::exit(3);
                            }
                        }
                    }
                }
            }
        }
        
        // Push to all remotes after updating SSH config
        if !dry_run && (iteration > 1 || initial_refresh) {
            let push_errors = git_push_all(&config_dir);
            for error in &push_errors {
                warn(&format!("[sync] {}", error));
                if verbose {
                    println!("  [sync] {}", error);
                }
            }
        }
        
        // Check if we found new devices by comparing current device files with previous
        let mut current_device_files: HashSet<String> = HashSet::new();
        if devices_dir.exists() {
            if let Ok(entries) = fs::read_dir(&devices_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        if entry.path().is_file() && entry.path().extension().map_or(false, |ext| ext == "json") {
                            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                                current_device_files.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        // Check if new device files appeared
        for new_file in &current_device_files {
            if !previous_device_files.contains(new_file) {
                new_devices_found = true;
                has_local_changes = true;
                break;
            }
        }
        
        // Update previous for next iteration
        previous_device_files = current_device_files;
    }
    
    DiscoveryResult { new: new_count, updated, missing }
}
