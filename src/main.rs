// regroup — find machines by their SSH host key fingerprint and update ~/.ssh/config.
//
// Configuration is stored in ~/.regroup-config/ git repository where each device
// has its own JSON file in the devices/ directory. Git remotes are automatically
// configured for all other devices to enable decentralized sync.
// NOTE: Configuration files are backups of discovered information, not the primary
// source of truth. Devices identify themselves via their public key (SSH fingerprint).
//
// Continuous interactive mode: listens for UDP beacons in background, prompts user
// for each new device. User can press 'r' to sync with git, 'q' to quit.
// Each device owns its own config file and can only overwrite it.
//
// Build (Termux):  pkg install rust openssh && cargo build --release
// Build (Ubuntu): sudo apt install cargo && cargo build --release
//
// Run:            ./regroup                (continuous interactive mode)
//                 ./regroup test           (dry run: nothing is modified)
//                 ./regroup -v             (verbose)
//                 ./regroup selftest       (built-in checks)
//                 ./regroup -h             (this help)
//
// Interactive commands:
//   y/N             - add or skip current device
//   r               - refresh git (pull from remotes, push to remotes)
//   q               - quit the program
//
// Exit codes: 0 = normal exit; 1 = selftest failure; 2 = configuration or environment error.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

mod cli;
mod config;
mod discovery;
mod git;
mod signing;
mod ssh;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use cli::{add_device_to_config, device_name_taken, die, refresh_git, run_selftest, setup_cd, unique_device_name, usage};
use config::{load_config, Machine};
use discovery::beacon::{announce_beacon, listen_beacons_continuous, BeaconDevice};
use ssh::config::update_block;
use ssh::keys::SshPublicKey;
use ssh::public_key;
use discovery::{get_local_interfaces, host_name, my_ip};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut help = false;
    let mut selftest = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "test" {
            dry_run = true;
        } else if a == "-v" || a == "--verbose" {
            verbose = true;
        } else if a == "-h" || a == "--help" {
            help = true;
        } else if a == "selftest" {
            selftest = true;
        } else if a == "setup-cd" {
            setup_cd();
        } else {
            eprintln!("ERROR: unknown option: {}", a);
            usage();
            std::process::exit(2);
        }
        i += 1;
    }
    if help {
        usage();
        return;
    }
    if selftest {
        run_selftest();
    }

    // --- external tools ---
    for tool in ["ssh-keyscan", "ssh-keygen"] {
        if Command::new(tool).output().is_err() {
            die(&format!(
                "command \"{}\" not found — Termux: pkg install openssh; Ubuntu: sudo apt install openssh-client",
                tool
            ));
        }
    }

    // --- configuration ---
    let conf = load_config();

    // --- ~/.ssh preparation (only when we may write) ---
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let ssh_dir = PathBuf::from(&home).join(".ssh");
    let ssh_config = ssh_dir.join("config");
    if !dry_run {
        if !ssh_dir.exists() && fs::create_dir(&ssh_dir).is_err() {
            die("cannot create ~/.ssh");
        }
        let _ = fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700));
        if !ssh_config.exists() && fs::write(&ssh_config, "").is_err() {
            die("cannot create ~/.ssh/config");
        }
        let _ = fs::set_permissions(&ssh_config, fs::Permissions::from_mode(0o600));
        if fs::OpenOptions::new()
            .append(true)
            .open(&ssh_config)
            .is_err()
        {
            die("~/.ssh/config is not writable");
        }
    }

    // --- local identity ---
    // IMPORTANT: We need the SSH *server* public key to uniquely identify this machine.
    // This is CRITICAL for self-detection filtering.
    // - Hostname CANNOT be used (Termux randomizes it on each launch)
    // - IP address CANNOT be used (changes with network)
    // - SSH public key IS PERSISTENT and uniquely identifies a machine
    // Without a valid SSH server public key, the device cannot be uniquely identified
    // and should NOT announce itself (as it cannot accept SSH connections anyway).
    let local_name = host_name();
    let local_ip = my_ip();
    
    // Get SSH server PUBLIC KEY (full key, not fingerprint) - this is the UNIQUE identifier for this machine
    // Returns None if no SSH server is installed/configured
    // NOTE: We use get_ssh_server_public_key() NOT get_ssh_server_fingerprint()
    // because the beacon format expects the full public key, not the fingerprint.
    let local_public_key = match public_key::get_ssh_server_public_key() {
        Some(key) => key,
        None => {
            // No SSH server public key found - device cannot accept SSH connections
            // and cannot be uniquely identified - do NOT announce
            eprintln!("WARNING: No SSH server host key found. This device will NOT announce itself.");
            eprintln!("         Other devices can still discover and connect to this machine if they have its IP.");
            eprintln!("         To fix: Install and configure an SSH server (e.g., 'pkg install openssh' in Termux).");
            SshPublicKey::from("") // Empty key signals "do not announce"
        }
    };

    if verbose {
        if !local_public_key.is_empty() {
            println!(
                "  . this machine: {} / {} (SSH key: {})",
                local_name,
                local_ip.as_deref().unwrap_or("unknown IP"),
                &local_public_key.as_str()[..16.min(local_public_key.as_str().len())]
            );
        } else {
            println!("  . this machine: {} / {} (NO SSH SERVER - not announcing)",
                local_name,
                local_ip.as_deref().unwrap_or("unknown IP"));
        }
    }

    // --- Continuous interactive mode ---
    
    // Create channel for beacon notifications
    let (tx, rx) = mpsc::channel::<BeaconDevice>();
    
    // Flag to signal threads to stop
    let running = Arc::new(AtomicBool::new(true));
    let running_listener = running.clone();
    let running_announcer = running.clone();
    
    // SSH port this machine's server listens on: taken from our own device config
    // (set by the user if the server does not use the default 22), 22 otherwise.
    let local_ssh_port = conf.machines
        .iter()
        .find(|m| m.name == local_name)
        .map(|m| m.port)
        .unwrap_or(22);

    // Start beacon listener in background (always listen for other devices)
    // Signature verification, beacon age check and IP consistency check are
    // performed inside listen_beacons_continuous; only valid beacons reach the queue.
    let listen_thread = start_beacon_listener(conf.udp_port, running_listener, tx.clone(), local_public_key.clone());
    
    // Start beacon announcer in background - only if we have a valid SSH server key
    // Without a server key, announcing is pointless as we cannot be contacted via SSH
    let announce_thread = if !local_public_key.is_empty() {
        // Load or generate signing key
        match crate::signing::initialize_signing_keys() {
            Ok((signing_key, _)) => {
                let ip_for_announcer = local_ip.clone().unwrap_or_else(|| "0.0.0.0".to_string());
                Some(start_beacon_announcer(
                    conf.udp_port,
                    conf.wait_secs,
                    local_name.clone(),
                    local_public_key.clone(),
                    ip_for_announcer,
                    local_ssh_port,
                    signing_key,
                    running_announcer,
                ))
            }
            Err(e) => {
                eprintln!("WARNING: Cannot initialize signing keys: {}", e);
                None
            }
        }
    } else {
        // No announcement thread - device has no SSH server
        eprintln!("INFO: Not announcing - no SSH server host key detected.");
        None
    };

    // Build lookup maps from config
    let mut known_keys: HashSet<SshPublicKey> = conf.machines
        .iter()
        .map(|m| m.ssh_public_key.clone())
        .collect();
    
    // Map from SSH public key to machine info for known devices
    let mut key_to_machine: HashMap<SshPublicKey, Machine> = conf.machines
        .iter()
        .map(|m| (m.ssh_public_key.clone(), m.clone()))
        .collect();
    
    // Queue for pending devices to confirm
    let mut pending_devices: VecDeque<BeaconDevice> = VecDeque::new();
    
    // Track pending devices by public key to prevent duplicates
    let mut pending_keys: HashSet<SshPublicKey> = HashSet::new();
    
    // Track last known IP for known devices to avoid redundant notifications
    let mut known_ip: HashMap<SshPublicKey, String> = HashMap::new();
    
    // Track rejected devices to avoid re-asking
    let mut rejected_keys: HashSet<SshPublicKey> = HashSet::new();
    
    // Statistics
    let mut new_count = 0;
    
    // Channel for user input (non-blocking)
    let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
    
    // Spawn stdin reader thread - reads all input and sends via channel
    let _stdin_thread = thread::spawn(move || {
        let mut input = String::new();
        loop {
            input.clear();
            match io::stdin().read_line(&mut input) {
                Ok(_) => {
                    if cmd_tx.send(input.clone()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // State for prompt display - FIFO queue for questions
    // Timer to control how often we re-display the current question prompt
    let mut last_prompt_time = std::time::Instant::now();
    const PROMPT_REPEAT_INTERVAL: Duration = Duration::from_secs(2);
    
    // Device waiting for the user to pick a new name (device, suggested name).
    // Set when the user accepts a device whose name is already taken; the next
    // input line is then interpreted as the new name instead of a command.
    let mut rename_state: Option<(BeaconDevice, String)> = None;
    
    // State for waiting prompt (to avoid duplicate waiting messages)
    let mut waiting_prompt_shown = false;
    
    // Main interactive loop
    println!("\nListening for devices on UDP port {}...", conf.udp_port);
    println!("Press 'r' to refresh git, 'q' to quit.");
    println!("New devices will be prompted for confirmation.\n");

    loop {
        // Check for new beacons from the listener
        match rx.try_recv() {
            Ok(device) => {
                // IMPORTANT: Filter out self-detection using SSH public key only.
                // The SSH public key is the unique identifier for a machine.
                // We cannot use hostname (Termux changes it) or IP (network changes).
                // If the device's SSH public key matches ours, this beacon is from us - ignore it.
                if device.ssh_public_key == local_public_key {
                    continue;
                }
                
                // Check if this is a known device
                if known_keys.contains(&device.ssh_public_key) {
                    // Check if IP changed before notifying
                    if let Some(last_ip) = known_ip.get(&device.ssh_public_key) {
                        if last_ip == &device.ip {
                            continue; // IP unchanged, skip silently
                        }
                    }
                    // Known device with new IP: update SSH config automatically
                    if let Some(machine) = key_to_machine.get(&device.ssh_public_key) {
                        if !dry_run {
                            let _ = update_block(
                                &ssh_config,
                                &machine.name,
                                &device.ip,
                                &machine.user,
                                machine.port,
                                machine.port_explicit,
                            );
                            known_ip.insert(device.ssh_public_key.clone(), device.ip.clone());
                            println!("\n[~] Known device: {} at {} (IP updated)", 
                                machine.name, device.ip);
                        }
                    }
                    continue; // Don't add to pending queue
                }
                
                // New device: check for duplicates in pending queue and rejected list
                if !pending_keys.contains(&device.ssh_public_key) && !rejected_keys.contains(&device.ssh_public_key) {
                    pending_keys.insert(device.ssh_public_key.clone());
                    pending_devices.push_back(device);
                    if !dry_run {
                        let added_device = pending_devices.back().unwrap();
                        let key_preview = if added_device.ssh_public_key.as_str().len() > 16 {
                            format!("{}...", &added_device.ssh_public_key.as_str()[..13])
                        } else {
                            added_device.ssh_public_key.to_string()
                        };
                        println!("\n[!] New device: {} ({}) - key: {}", 
                            added_device.name, added_device.ip, key_preview);
                        println!("    Type 'y' to add, 'N' to skip, 'r' to refresh git, 'q' to quit");
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                break;
            }
        }

        // Check for user commands from stdin
        let user_command = cmd_rx.try_recv();
        
        // If we are waiting for the user to pick a new name for a device,
        // the next input line is that name (empty = suggested name).
        // Only reachable in interactive mode: dry run never adds devices.
        if rename_state.is_some() {
            if user_command.is_err() && last_prompt_time.elapsed() >= PROMPT_REPEAT_INTERVAL {
                let (device, suggested) = rename_state.as_ref().unwrap();
                print!("\nNew name for {} (empty = {}): ", device.name, suggested);
                io::stdout().flush().ok();
                last_prompt_time = std::time::Instant::now();
            }
            if let Ok(input) = user_command {
                last_prompt_time = std::time::Instant::now();
                let input = input.trim();
                if input == "q" {
                    println!("  Skipped {}.", rename_state.as_ref().unwrap().0.name);
                    rename_state = None;
                } else {
                    let (device, suggested) = rename_state.take().unwrap();
                    let name = if input.is_empty() { suggested } else { input.to_string() };
                    if device_name_taken(&name, &home, &ssh_config) {
                        println!("  ! '{}' is also already used by another machine or an existing ~/.ssh/config entry.", name);
                        let new_suggested = unique_device_name(&name, &home, &ssh_config);
                        println!("    Pick another name (empty = {}):", new_suggested);
                        rename_state = Some((device, new_suggested));
                    } else if let Some(machine) = add_device_to_config(&home, &ssh_config, &device, &name, &mut new_count) {
                        // Remember the machine under its final name so future
                        // beacons update the right ~/.ssh/config block.
                        known_keys.insert(device.ssh_public_key.clone());
                        key_to_machine.insert(device.ssh_public_key.clone(), machine);
                        known_ip.insert(device.ssh_public_key.clone(), device.ip.clone());
                    }
                }
            }
        } else
        // Process pending devices if any
        if !pending_devices.is_empty() {
            if !dry_run {
                // Show prompt for the current device (first in queue) - FIFO
                // Re-display prompt periodically (every PROMPT_REPEAT_INTERVAL) until answered
                if user_command.is_err() && last_prompt_time.elapsed() >= PROMPT_REPEAT_INTERVAL {
                    let device = &pending_devices[0];
                    print!("\nAdd {} to config? [y/N/q/r]: ", device.name);
                    io::stdout().flush().ok();
                    last_prompt_time = std::time::Instant::now();
                }
                
                // Process user command if available
                if let Ok(input) = user_command {
                    last_prompt_time = std::time::Instant::now();
                    let device = pending_devices.pop_front().unwrap();
                    let device_name = device.name.clone();
                    let device_ssh_public_key = device.ssh_public_key.clone();
                    pending_keys.remove(&device_ssh_public_key);
                    let input = input.trim().to_lowercase();
                    match input.as_str() {
                        "y" | "yes" => {
                            // If the name is already used by another machine,
                            // ask the user to pick a new one.
                            if device_name_taken(&device_name, &home, &ssh_config) {
                                println!("\n  ! Name '{}' is already used by another machine or an existing ~/.ssh/config entry.", device_name);
                                let suggested = unique_device_name(&device_name, &home, &ssh_config);
                                print!("  New name for this device (empty = {}): ", suggested);
                                io::stdout().flush().ok();
                                last_prompt_time = std::time::Instant::now();
                                rename_state = Some((device, suggested));
                            } else if let Some(machine) = add_device_to_config(&home, &ssh_config, &device, &device_name, &mut new_count) {
                                known_keys.insert(device_ssh_public_key.clone());
                                key_to_machine.insert(device_ssh_public_key.clone(), machine);
                                known_ip.insert(device_ssh_public_key.clone(), device.ip.clone());
                            }
                        }
                        "r" => {
                            pending_devices.push_front(device);
                            pending_keys.insert(device_ssh_public_key);
                            refresh_git(&home);
                            // A pull may have brought new device configs: reload
                            reload_known_devices(&mut known_keys, &mut key_to_machine);
                        }
                        "q" => {
                            println!();
                            break;
                        }
                        _ => {
                            // Device rejected, remember to not ask again
                            rejected_keys.insert(device_ssh_public_key);
                            println!("  Skipped {}.", device_name);
                        }
                    }
                }
            } else {
                // Dry run: show all devices as they are detected
                let device = pending_devices.pop_front().unwrap();
                pending_keys.remove(&device.ssh_public_key);
                let key_preview = if device.ssh_public_key.as_str().len() > 16 {
                    format!("{}...", &device.ssh_public_key.as_str()[..13])
                } else {
                    device.ssh_public_key.to_string()
                };
                println!("\n[DRY RUN] New device: {} ({}) - key: {}", 
                    device.name, device.ip, key_preview);
                new_count += 1;
            }
        } else {
            // No pending devices
            if let Ok(input) = user_command {
                let input = input.trim().to_lowercase();
                match input.as_str() {
                    "q" | "quit" => {
                        println!();
                        println!("Stopping...");
                        break;
                    }
                    "r" | "refresh" => {
                        println!();
                        // Dry run must not modify anything: skip git refresh
                        if dry_run {
                            println!("  (dry run - git refresh skipped)");
                        } else {
                            refresh_git(&home);
                            // A pull may have brought new device configs: reload
                            reload_known_devices(&mut known_keys, &mut key_to_machine);
                        }
                    }
                    _ => {}
                }
                waiting_prompt_shown = false;
            } else if !waiting_prompt_shown {
                print!("Waiting for devices or command [q=quit, r=refresh git]: ");
                io::stdout().flush().ok();
                waiting_prompt_shown = true;
            }
        }
        
        // Small delay to prevent busy waiting
        thread::sleep(Duration::from_millis(100));
    }
    
    // Clean up threads
    println!("Waiting for beacon threads to stop...");
    drop(tx);
    running.store(false, Ordering::Relaxed);
    
    // Wait for threads with timeout
    let timeout = Duration::from_secs(2);
    join_with_timeout(listen_thread, "Listener", timeout);
    
    // Only join announce thread if it exists (it won't exist if we have no SSH server key)
    if let Some(thread) = announce_thread {
        join_with_timeout(thread, "Announcer", timeout);
    }
    
    println!("Done.");
    
    // Calculate final statistics
    // missing = original machines that were not seen
    // For now, we only track new (added). Updated and missing would require more complex tracking.
    // Since we're in continuous mode, we can't easily detect missing without periodic scans.
    // For updated, we'd need to track IP changes, which we're not currently doing.
    
    println!("\n== {} new, 0 updated, 0 missing ==", new_count);
}

/// Start beacon listener thread - continuous mode with short timeout.
/// Delegates to listen_beacons_continuous, which verifies each beacon's
/// signature, age and IP consistency before forwarding it.
fn start_beacon_listener(
    port: u16,
    running: Arc<AtomicBool>,
    sender: mpsc::Sender<BeaconDevice>,
    local_ssh_public_key: SshPublicKey,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        listen_beacons_continuous(port, sender, local_ssh_public_key, running);
    })
}

/// Start beacon announcer thread
fn start_beacon_announcer(
    port: u16,
    secs: u64,
    name: String,
    ssh_public_key: SshPublicKey,
    ip: String,
    ssh_port: u16,
    signing_key: ed25519_dalek::SigningKey,
    running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    let interfaces = get_local_interfaces();
    thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            announce_beacon(port, secs, &name, &ssh_public_key, &ip, ssh_port, &signing_key, &interfaces, Some(&running));
            std::thread::sleep(Duration::from_secs(2));
        }
    })
}

/// Reload device configuration from disk (e.g. after a git refresh pulled new
/// device files) and rebuild the lookup maps used by the interactive loop.
fn reload_known_devices(
    known_keys: &mut HashSet<SshPublicKey>,
    key_to_machine: &mut HashMap<SshPublicKey, Machine>,
) {
    let conf = load_config();
    *known_keys = conf.machines.iter().map(|m| m.ssh_public_key.clone()).collect();
    *key_to_machine = conf.machines
        .iter()
        .map(|m| (m.ssh_public_key.clone(), m.clone()))
        .collect();
}

/// Wait for a thread to finish, at most `timeout`.
/// Warns if the thread is still running when the timeout expires; the thread
/// is then left to finish on its own (the process exits when main returns).
fn join_with_timeout(handle: thread::JoinHandle<()>, name: &str, timeout: Duration) {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let _waiter = thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });
    if done_rx.recv_timeout(timeout).is_err() {
        eprintln!("  {} thread did not stop within {}s", name, timeout.as_secs());
    }
}

