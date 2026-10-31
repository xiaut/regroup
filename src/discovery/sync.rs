//! Discovery and sync loop module
//!
//! This module contains the main discovery and synchronization loop.
//!
//! Critical design note: SSH config (~/.ssh/config) MUST be updated BEFORE
//! any git operations, because git uses SSH to connect to remotes and
//! requires the SSH config to know how to reach each machine.
//!
//! Order of operations:
//! 1. Discovery (UDP beacons + TCP scan) - find machines and their IPs
//! 2. Update SSH config - so git knows how to connect to each machine
//! 3. Refresh git remotes - ensure git knows about all machines
//! 4. Git sync (pull/push) - exchange configuration files

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;

use super::{announce_beacon, fingerprints, listen_beacons, net_prefix, net_valid, scan_subnet};
use crate::cli::{ask_conflict_resolution, die, warn};
use crate::config::{Conf, CONFIG_DIR, DEVICES_DIR, GLOBAL_CONFIG_FILE, MAX_SYNC_ITERATIONS, load_config, read_device_file_content};
use crate::git::{git_add_and_commit_file, get_modified_config_files, git_push_all, has_unstaged_changes, is_git_repo, refresh_git_remotes, sync_with_remotes};
use crate::ssh::{flush_ssh_config, update_block, verify_ssh_config_complete};

/// Log the current phase of the sync process
fn log_phase(verbose: bool, iteration: usize, phase: u8, total_phases: u8, message: &str) {
    if verbose || iteration == 1 {
        println!("[{}={}] {}", phase, total_phases, message);
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
    net_arg: &Option<String>,
) -> (usize, usize) {
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
    let mut missing = 0;
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
        // This allows immediate use of modified settings (e.g., new subnets) without requiring a commit
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
        // Find machines and their current IPs via UDP beacons and TCP scanning
        // This does NOT require SSH to be configured
        // ============================================================
        if verbose {
            println!("== Iteration {}: Announcing + listening for beacons ({} s, UDP port {}) ==",
                iteration, current_conf.wait_secs, current_conf.udp_port);
        } else if iteration == 1 {
            println!(
                "== Announcing + listening for beacons ({} s, UDP port {}) ==",
                current_conf.wait_secs, current_conf.udp_port
            );
        }
        
        let announcer = match local_ip.clone() {
            Some(ip) => {
                let port = current_conf.udp_port;
                let secs = current_conf.wait_secs;
                let name = local_name.to_string();
                Some(thread::spawn(move || {
                    announce_beacon(port, secs, &name, &ip)
                }))
            }
            None => {
                if iteration == 1 {
                    warn("local IP unknown — announcing disabled (listening continues)");
                }
                None
            }
        };
        
        let beacons = listen_beacons(current_conf.udp_port, current_conf.wait_secs);
        if let Some(h) = announcer {
            let _ = h.join();
        }
        
        if verbose {
            println!("  . {} beacon(s) received", beacons.len());
            for (n, ip) in &beacons {
                println!(
                    "  . beacon: {} ({})",
                    if n.is_empty() { "?" } else { n.as_str() },
                    ip
                );
            }
        }
        
        // --- candidates: beacons x ports ---
        let mut ports: Vec<u16> = current_conf.machines.iter().map(|m| m.port).collect();
        ports.sort_unstable();
        ports.dedup();

        let mut candidates: Vec<(String, u16)> = Vec::new();
        for (_, ip) in &beacons {
            for p in &ports {
                candidates.push((ip.clone(), *p));
            }
        }

        // --- fallback (or requested) subnet scan ---
        let mut nets: Vec<String> = Vec::new();
        match net_arg {
            Some(a) => {
                if let Some(n) = net_valid(&net_prefix(a)) {
                    nets.push(n);
                } else {
                    die(&format!(
                        "argument \"{}\" is not a valid /24 prefix (e.g. 192.168.43)",
                        a
                    ));
                }
            },
            None => nets.extend(current_conf.subnets.iter().cloned()),
        }
        let force_scan = net_arg.is_some();
        if (candidates.is_empty() || force_scan) && !nets.is_empty() {
            if iteration == 1 {
                println!(
                    "== Subnet scan ({}) ==",
                    if force_scan { "requested" } else { "fallback" }
                );
            }
            for net in &nets {
                if verbose {
                    println!("  scanning {}.0/24 (port {}) ...", net, ports[0]);
                }
                for p in &ports {
                    candidates.extend(scan_subnet(net, *p, current_conf.batch_size));
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        if verbose {
            println!("  . {} candidate(s) in total", candidates.len());
            for (ip, p) in &candidates {
                println!("  . candidate {}:{}", ip, p);
            }
        }
        
        // --- fingerprints of the candidates ---
        if iteration == 1 {
            println!("== Fingerprinting candidates ==");
        }
        let md5_needed = current_conf
            .machines
            .iter()
            .any(|m| m.fingerprint.starts_with("MD5:"));
        let mut seen: Vec<(String, u16, String)> = Vec::new();
        for (ip, port) in &candidates {
            let fps = fingerprints(ip, *port, false);
            if fps.is_empty() {
                warn(&format!(
                    "port open on {}:{} but no SSH key received (not an SSH server?)",
                    ip, port
                ));
                continue;
            }
            for f in fps {
                seen.push((ip.clone(), *port, f));
            }
            if md5_needed {
                for f in fingerprints(ip, *port, true) {
                    seen.push((ip.clone(), *port, f));
                }
            }
        }
        if verbose {
            println!("  . {} fingerprint(s) collected", seen.len());
            for (ip, p, f) in &seen {
                println!("  . fingerprint {}:{} {}", ip, p, f);
            }
        }
        if candidates.is_empty() && iteration == 1 {
            warn(
                "no beacons received and no open ports found — run regroup on the other machine during the window, or pass a subnet: ./regroup 192.168.43",
            );
        } else if seen.is_empty() && iteration == 1 {
            warn("open ports found but no SSH server responded");
        }

        // --- fingerprints of this machine's own IP, for local detection ---
        let mut local_fps: Vec<(u16, String)> = Vec::new();
        if let Some(ip) = local_ip {
            for p in &ports {
                for f in fingerprints(ip, *p, false) {
                    local_fps.push((*p, f));
                }
                if md5_needed {
                    for f in fingerprints(ip, *p, true) {
                        local_fps.push((*p, f));
                    }
                }
            }
        }

        // ============================================================
        // PHASE 2: Update SSH config
        // ============================================================
        // Update ~/.ssh/config with discovered IPs so that git can connect
        // This MUST happen BEFORE any git operations (pull/push)
        // ============================================================
        if iteration == 1 || verbose {
            log_phase(verbose, iteration, 2, 4, "Updating SSH config (~/.ssh/config)");
        }
        
        let mut ssh_updated = 0;
        let mut ssh_failed: Vec<String> = Vec::new();
        
        // --- machines ---
        if iteration == 1 {
            println!("== Machines ==");
        }
        let mut iteration_found = 0;
        let mut iteration_missing = 0;
        
        for m in &current_conf.machines {
            // (a) pre-check: this machine offers the configured key itself
            if local_fps
                .iter()
                .any(|(p, f)| *p == m.port && *f == m.fingerprint)
            {
                if iteration == 1 {
                    println!(
                        "  o {}: this is the local machine (this host), config unchanged",
                        m.name
                    );
                }
                // Don't count local machine in the found total
                continue;
            }
            // resolution by fingerprint among the candidates
            let mut matched: Option<String> = None;
            for (ip, p, f) in &seen {
                if *p == m.port && *f == m.fingerprint {
                    matched = Some(ip.clone());
                    break;
                }
            }
            match matched {
                None => {
                    let servers = seen.iter().filter(|(_, p, _)| *p == m.port).count();
                    if servers > 0 && iteration == 1 {
                        println!(
                            "  x {}: {} SSH server(s) seen on port {}, none with a matching key",
                            m.name, servers, m.port
                        );
                    } else if iteration == 1 {
                        println!(
                            "  x {}: not found (no SSH server on port {})",
                            m.name, m.port
                        );
                    }
                    iteration_missing += 1;
                }
                Some(ip) => {
                    // (b) the resolved machine is this machine
                    if local_ip.as_deref() == Some(ip.as_str()) {
                        if iteration == 1 {
                            println!(
                                "  o {}: this is the local machine (this host), config unchanged",
                                m.name
                            );
                        }
                        // Don't count local machine in the found total
                    } else if dry_run {
                        if iteration == 1 {
                            println!("  ~ {} -> {} (test mode, config not modified)", m.name, ip);
                        }
                        iteration_found += 1;
                    } else {
                        match update_block(&ssh_config, &m.name, &ip, &m.user, m.port, m.port_explicit)
                        {
                            Ok(()) => {
                                if iteration == 1 {
                                    println!("  + {} -> {}  (~/.ssh/config updated)", m.name, ip);
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
                }
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
        
        found = iteration_found;
        missing = iteration_missing;
        
        // ============================================================
        // PHASE 3: Git remotes configuration
        // ============================================================
        // Configure git remotes for all machines
        // Requires SSH config to be up to date (Phase 2)
        // ============================================================
        if !dry_run {
            log_phase(verbose, iteration, 3, 4, "Configuring git remotes");
            
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
            if iteration > 1 || initial_refresh {
                log_phase(verbose, iteration, 4, 4, "Syncing with git remotes (pull/push)");
                
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
    
    (found, missing)
}
