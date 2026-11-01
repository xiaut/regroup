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
// Exit codes: 0 = normal exit; 2 = configuration or environment error.

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

use cli::{add_device_to_config, die, refresh_git, run_selftest, setup_cd, usage};
use config::{load_config, Machine};
use discovery::beacon::{announce_beacon, BeaconDevice};
use ssh::config::update_block;
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
    
    // Get SSH server public key - this is the UNIQUE identifier for this machine
    // Returns None if no SSH server is installed/configured
    let local_public_key = match public_key::get_ssh_server_fingerprint() {
        Some(key) => key,
        None => {
            // No SSH server public key found - device cannot accept SSH connections
            // and cannot be uniquely identified - do NOT announce
            eprintln!("WARNING: No SSH server host key found. This device will NOT announce itself.");
            eprintln!("         Other devices can still discover and connect to this machine if they have its IP.");
            eprintln!("         To fix: Install and configure an SSH server (e.g., 'pkg install openssh' in Termux).");
            String::new() // Empty string signals "do not announce"
        }
    };

    if verbose {
        if !local_public_key.is_empty() {
            println!(
                "  . this machine: {} / {} (SSH key: {})",
                local_name,
                local_ip.as_deref().unwrap_or("unknown IP"),
                &local_public_key[..16.min(local_public_key.len())]
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
    
    // Start beacon listener in background (always listen for other devices)
    let listen_thread = start_beacon_listener(conf.udp_port, running_listener, tx.clone());
    
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
                    22, // SSH port
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
    let mut known_keys: HashSet<String> = conf.machines
        .iter()
        .map(|m| m.ssh_public_key.clone())
        .collect();
    
    // Map from SSH public key to machine info for known devices
    let mut key_to_machine: HashMap<String, Machine> = conf.machines
        .iter()
        .map(|m| (m.ssh_public_key.clone(), m.clone()))
        .collect();
    
    // Queue for pending devices to confirm
    let mut pending_devices: VecDeque<BeaconDevice> = VecDeque::new();
    
    // Track pending devices by public key to prevent duplicates
    let mut pending_keys: HashSet<String> = HashSet::new();
    
    // Track last known IP for known devices to avoid redundant notifications
    let mut known_ip: HashMap<String, String> = HashMap::new();
    
    // Track rejected devices to avoid re-asking
    let mut rejected_keys: HashSet<String> = HashSet::new();
    
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
                        let key_preview = if added_device.ssh_public_key.len() > 16 {
                            format!("{}...", &added_device.ssh_public_key[..13])
                        } else {
                            added_device.ssh_public_key.clone()
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
                            // Add to known keys so future beacons will auto-update IP
                            known_keys.insert(device_ssh_public_key.clone());
                            let machine = Machine {
                                name: device_name.clone(),
                                ssh_public_key: device_ssh_public_key.clone(),
                                user: String::new(),
                                port: device.port,
                                port_explicit: false,
                            };
                            key_to_machine.insert(device_ssh_public_key.clone(), machine);
                            known_ip.insert(device_ssh_public_key.clone(), device.ip.clone());
                            add_device_to_config(&home, &ssh_config, &device, &mut known_keys, &mut new_count);
                        }
                        "r" => {
                            pending_devices.push_front(device);
                            pending_keys.insert(device_ssh_public_key);
                            refresh_git(&home);
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
                let key_preview = if device.ssh_public_key.len() > 16 {
                    format!("{}...", &device.ssh_public_key[..13])
                } else {
                    device.ssh_public_key.clone()
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
                        refresh_git(&home);
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
    let start = std::time::Instant::now();
    
    let _ = listen_thread.join().map_err(|_| {
        if start.elapsed() >= timeout {
            eprintln!("  Listener thread did not stop cleanly");
        }
    });
    
    // Only join announce thread if it exists (it won't exist if we have no SSH server key)
    if let Some(thread) = announce_thread {
        let _ = thread.join().map_err(|_| {
            if start.elapsed() >= timeout {
                eprintln!("  Announcer thread did not stop cleanly");
            }
        });
    }
    
    println!("Done.");
    
    // Calculate final statistics
    // missing = original machines that were not seen
    // For now, we only track new (added). Updated and missing would require more complex tracking.
    // Since we're in continuous mode, we can't easily detect missing without periodic scans.
    // For updated, we'd need to track IP changes, which we're not currently doing.
    
    println!("\n== {} new, 0 updated, 0 missing ==", new_count);
}

/// Start beacon listener thread - continuous mode with short timeout
fn start_beacon_listener(
    port: u16,
    running: Arc<AtomicBool>,
    sender: mpsc::Sender<BeaconDevice>
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let sock = match std::net::UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("WARNING: cannot listen on UDP port {} ({}) — beacons ignored", port, e);
                return;
            }
        };
        let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
        let mut buf = [0u8; 4096];
        
        while running.load(Ordering::Relaxed) {
            match sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let parts: Vec<&str> = text.split('\t').collect();
                    
                    // Format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
                    if parts.len() == 7 {
                        let device = BeaconDevice {
                            name: parts[0].to_string(),
                            signing_public_key: parts[1].to_string(),
                            ssh_public_key: parts[2].to_string(),
                            ip: parts[3].to_string(),
                            port: parts[4].parse().unwrap_or(22),
                            timestamp: parts[5].to_string(),
                            signature: parts[6].to_string(),
                        };
                        if sender.send(device).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    })
}

/// Start beacon announcer thread
fn start_beacon_announcer(
    port: u16,
    secs: u64,
    name: String,
    ssh_public_key: String,
    ip: String,
    ssh_port: u16,
    signing_key: ed25519_dalek::SigningKey,
    running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    let interfaces = get_local_interfaces();
    thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            announce_beacon(port, secs, &name, &ssh_public_key, &ip, ssh_port, &signing_key, &interfaces);
            std::thread::sleep(Duration::from_secs(2));
        }
    })
}

