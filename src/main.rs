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
mod ssh;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use cli::{add_device_to_config, die, refresh_git, run_selftest, setup_cd, usage};
use config::load_config;
use discovery::beacon::{announce_beacon, BeaconDevice};
use discovery::{get_local_interfaces, host_name, my_ip};
use discovery::local::get_local_fingerprint;

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
    let local_name = host_name();
    let local_ip = my_ip();
    let local_ssh_port = conf.machines
        .iter()
        .find(|m| m.name == local_name)
        .map(|m| m.port)
        .unwrap_or(22);
    let local_public_key = get_local_fingerprint(local_ssh_port).unwrap_or_default();

    if verbose {
        println!(
            "  . this machine: {} / {} (key: {})",
            local_name,
            local_ip.as_deref().unwrap_or("unknown IP"),
            &local_public_key[..16.min(local_public_key.len())]
        );
    }

    // --- Continuous interactive mode ---
    
    // Create channel for beacon notifications
    let (tx, rx) = mpsc::channel::<BeaconDevice>();
    
    // Start beacon listener in background
    let listen_thread = start_beacon_listener(conf.udp_port, conf.wait_secs, tx.clone());
    
    // Start beacon announcer in background
    let announce_thread = start_beacon_announcer(
        conf.udp_port,
        conf.wait_secs,
        local_name.clone(),
        local_public_key.clone(),
    );

    // Load known public keys from config
    let mut known_keys: std::collections::HashSet<String> = conf.machines
        .iter()
        .map(|m| m.fingerprint.clone())
        .collect();
    
    // Queue for pending devices to confirm
    let mut pending_devices: VecDeque<BeaconDevice> = VecDeque::new();
    
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

    // State for prompt display (to avoid duplicate prompts)
    let mut prompt_shown = false;
    
    // Main interactive loop
    println!("\nListening for devices on UDP port {}...", conf.udp_port);
    println!("Press 'r' to refresh git, 'q' to quit.");
    println!("New devices will be prompted for confirmation.\n");

    loop {
        // Check for new beacons from the listener
        match rx.try_recv() {
            Ok(device) => {
                // Check if this is us or already known
                if device.public_key == local_public_key || known_keys.contains(&device.public_key) {
                    continue; // Skip known devices and self
                }
                // Notify immediately when a new device is detected
                pending_devices.push_back(device);
                if !dry_run {
                    let device = pending_devices.back().unwrap();
                    println!("\n[!] New device: {} ({}) - key: {}", 
                        device.name, device.ip, &device.public_key[..16.min(device.public_key.len())]);
                    println!("    Type 'y' to add, 'N' to skip, 'r' to refresh git, 'q' to quit");
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
                // Show prompt for the current device (first in queue)
                if !prompt_shown && user_command.is_err() {
                    let device = &pending_devices[0];
                    print!("\nAdd {} to config? [y/N/q/r]: ", device.name);
                    io::stdout().flush().ok();
                    prompt_shown = true;
                }
                
                // Process user command if available
                if let Ok(input) = user_command {
                    prompt_shown = false;
                    let device = pending_devices.pop_front().unwrap();
                    let input = input.trim().to_lowercase();
                    match input.as_str() {
                        "y" | "yes" => {
                            add_device_to_config(&home, &ssh_config, &device, &mut known_keys, &mut new_count);
                        }
                        "r" => {
                            pending_devices.push_front(device);
                            refresh_git(&home);
                        }
                        "q" => {
                            println!();
                            break;
                        }
                        _ => {
                            println!("  Skipped {}.", device.name);
                        }
                    }
                }
            } else {
                // Dry run: show all devices as they are detected
                let device = pending_devices.pop_front().unwrap();
                println!("\n[DRY RUN] New device: {} ({}) - key: {}", 
                    device.name, device.ip, &device.public_key[..16.min(device.public_key.len())]);
                new_count += 1;
            }
        } else {
            // No pending devices
            if let Ok(input) = user_command {
                let input = input.trim().to_lowercase();
                match input.as_str() {
                    "q" | "quit" => {
                        println!();
                        break;
                    }
                    "r" | "refresh" => {
                        println!();
                        refresh_git(&home);
                    }
                    _ => {}
                }
                prompt_shown = false;
            } else if !prompt_shown {
                print!("Waiting for devices or command [q=quit, r=refresh git]: ");
                io::stdout().flush().ok();
                prompt_shown = true;
            }
        }
        
        // Small delay to prevent busy waiting
        thread::sleep(Duration::from_millis(100));
    }
    
    // Clean up threads
    drop(tx);
    let _ = listen_thread.join();
    let _ = announce_thread.join();
    
    // Calculate final statistics
    // missing = original machines that were not seen
    // For now, we only track new (added). Updated and missing would require more complex tracking.
    // Since we're in continuous mode, we can't easily detect missing without periodic scans.
    // For updated, we'd need to track IP changes, which we're not currently doing.
    
    println!("\n== {} new, 0 updated, 0 missing ==", new_count);
}

/// Start beacon listener thread - continuous mode with short timeout
fn start_beacon_listener(port: u16, _secs: u64, sender: mpsc::Sender<BeaconDevice>) -> thread::JoinHandle<()> {
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
        
        loop {
            match sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let parts: Vec<&str> = text.split('\t').collect();
                    if parts.len() >= 3 {
                        let device = BeaconDevice {
                            name: parts[0].to_string(),
                            public_key: parts[1].to_string(),
                            ip: parts[2].to_string(),
                        };
                        let _ = sender.send(device);
                    }
                }
                Err(_) => {
                    // Timeout, continue looping
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    })
}

/// Start beacon announcer thread
fn start_beacon_announcer(port: u16, secs: u64, name: String, public_key: String) -> thread::JoinHandle<()> {
    let interfaces = get_local_interfaces();
    thread::spawn(move || {
        loop {
            announce_beacon(port, secs, &name, &public_key, &interfaces);
        }
    })
}

