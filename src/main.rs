// regroup — find machines by their SSH host key fingerprint and update ~/.ssh/config.
//
// Configuration is stored in ~/.regroup-config/ git repository where each device
// owns a directory devices/<name>/ (device.json, its known users, its
// connections). Git remotes are automatically configured for all other devices
// to enable decentralized sync; on conflicts the most recent commit wins.
// NOTE: Configuration files are backups of discovered information, not the primary
// source of truth. Devices identify themselves via their public key (SSH fingerprint).
//
// Continuous interactive mode: listens for UDP beacons in background, prompts user
// for each new device. User can press 's' to sync with git, 'q' to quit.
// Each device only writes inside its own directory (connection and user
// files); device.json holds the latest known state of the device.
//
// Build (Termux):  pkg install rust openssh && cargo build --release
// Build (Ubuntu): sudo apt install cargo && cargo build --release
//
// Run:            ./regroup                (continuous interactive mode)
//                 ./regroup test           (dry run: nothing is modified)
//                 ./regroup remove <name>  (remove a device)
//                 ./regroup wipe [--yes]   (delete the whole configuration)
//                 ./regroup -v             (verbose: toggle at runtime with 'v' in interactive mode)
//                 ./regroup selftest       (external tools + ssh-keygen fingerprint checks)
//                 ./regroup receive-install-key (internal: install SSH key on target machine)
//                 ./regroup receive-remove-key  (internal: remove SSH key from target machine)
//                 ./regroup -h             (this help)
//
// Interactive commands:
//   y/N             - add or skip current device
//   s               - sync git (pull from remotes, push to remotes)
//   d               - delete a known device (panel selection, Enter confirms)
//   w               - wipe the whole configuration (Enter confirms)
//   i               - install SSH key to a machine (first known machine)
//   r               - remove SSH key from a machine (first outgoing connection)
//   v               - toggle verbose mode (identity + git sync details in the activity log)
//   ?               - show/hide the help panel (full-screen mode)
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
mod interactive;
mod signing;
mod ssh;
mod tui;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use cli::{
    confirm, die, remove_device, run_selftest, setup_cd, usage, wipe as wipe_config,
};
use config::load_config;
use discovery::beacon::{BeaconDevice, BeaconIdentity, announce_beacon, listen_beacons_continuous};
use discovery::{get_local_interfaces, host_name, my_ip};
use ssh::keys::SshPublicKey;
use ssh::public_key;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut help = false;
    let mut selftest = false;
    let mut remove_name: Option<String> = None;
    let mut wipe = false;
    let mut assume_yes = false;
    let mut cli_mode = false;
    
    // Handle subcommands that don't need the full initialization
    if !args.is_empty() {
        match args[0].as_str() {
            "receive-install-key" => {
                cli::receive_install_key::run();
            }
            "receive-remove-key" => {
                cli::receive_remove_key::run();
            }
            _ => {}
        }
    }
    
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
            let home = match env::var("HOME") {
                Ok(h) => h,
                Err(_) => die("HOME variable not set"),
            };
            match setup_cd(&home) {
                Ok(rc) => {
                    let file = rc.file_name().unwrap_or_default().to_string_lossy();
                    println!("✓ 'regroup-cd' function installed in ~/{file}");
                    println!("Run 'source ~/{file}' or open a new terminal to start using it.");
                    println!("Use: regroup-cd");
                    std::process::exit(0);
                }
                Err(e) => die(&e),
            }
        } else if a == "remove" {
            remove_name = match args.get(i + 1) {
                Some(name) => {
                    i += 1;
                    Some(name.clone())
                }
                None => {
                    eprintln!("ERROR: remove needs a device name");
                    usage();
                    std::process::exit(2);
                }
            };
        } else if a == "wipe" {
            wipe = true;
        } else if a == "--yes" || a == "-y" {
            assume_yes = true;
        } else if a == "--cli" {
            cli_mode = true;
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

    // --- device removal / full wipe (non-interactive) ---
    if let Some(name) = &remove_name {
        let machine = conf
            .machines
            .iter()
            .find(|m| &m.name == name)
            .cloned()
            .unwrap_or_else(|| die(&format!("no device named \"{name}\" in the configuration")));
        if !assume_yes
            && !confirm(&format!(
                "Remove {}? (device file, ~/.ssh/config block, git remote)",
                machine.name
            ))
        {
            println!("Aborted.");
            return;
        }
        if remove_device(&home, &ssh_config, &machine, None) {
            println!("Removed {}.", machine.name);
        } else {
            std::process::exit(2);
        }
        return;
    }
    if wipe {
        if !assume_yes
            && !confirm(
                "Delete the entire regroup configuration and all device blocks in ~/.ssh/config?",
            )
        {
            println!("Aborted.");
            return;
        }
        wipe_config(&home, &ssh_config, &conf.machines);
        return;
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
            eprintln!(
                "WARNING: No SSH server host key found. This device will NOT announce itself."
            );
            eprintln!(
                "         Other devices can still discover and connect to this machine if they have its IP."
            );
            eprintln!(
                "         To fix: Install and configure an SSH server (e.g., 'pkg install openssh' in Termux)."
            );
            SshPublicKey::from("") // Empty key signals "do not announce"
        }
    };

    // --- Continuous interactive mode ---

    // Create channel for beacon notifications
    let (tx, rx) = mpsc::channel::<BeaconDevice>();

    // Flag to signal threads to stop
    let running = Arc::new(AtomicBool::new(true));
    let running_listener = running.clone();
    let running_announcer = running.clone();

    // SSH port and user this machine's server listens on: taken from our own device config
    // (set by the user if the server does not use the default 22), 22 otherwise.
    let local_machine = conf.machines.iter().find(|m| m.name == local_name);
    let local_ssh_port = local_machine.map(|m| m.port).unwrap_or(22);
    let local_user = local_machine.map(|m| m.user.clone()).unwrap_or_else(|| {
        std::env::var("USER").unwrap_or_else(|_| std::env::var("LOGNAME").unwrap_or_default())
    });

    // Start beacon listener in background (always listen for other devices)
    // Signature verification, beacon age check and IP consistency check are
    // performed inside listen_beacons_continuous; only valid beacons reach the queue.
    let listen_thread = start_beacon_listener(
        conf.udp_port,
        running_listener,
        tx.clone(),
        local_public_key.clone(),
    );

    // Start beacon announcer in background - only if we have a valid SSH server key
    // Without a server key, announcing is pointless as we cannot be contacted via SSH
    let announce_thread = if !local_public_key.is_empty() {
        // Load or generate signing key
        match crate::signing::initialize_signing_keys() {
            Ok((signing_key, _)) => {
                let ip_for_announcer = local_ip.clone().unwrap_or_else(|| "0.0.0.0".to_string());
                let identity = BeaconIdentity {
                    name: local_name.clone(),
                    ssh_public_key: local_public_key.clone(),
                    user: local_user.clone(),
                    ip: ip_for_announcer,
                    ssh_port: local_ssh_port,
                    signing_key,
                };
                Some(start_beacon_announcer(
                    identity,
                    conf.udp_port,
                    conf.wait_secs,
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

    // Run the interactive loop until the user quits (or the listener dies).
    let new_count = interactive::run(
        &interactive::Context {
            home: &home,
            ssh_config: &ssh_config,
            dry_run,
            local_public_key: &local_public_key,
            verbose,
            cli_mode,
            local_name: &local_name,
            local_ip: local_ip.as_deref(),
        },
        conf.udp_port,
        &rx,
    );

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

    // Only devices added this run are tracked; detecting updates or missing
    // devices would require periodic scans the continuous mode does not do.
    println!("\n== {} new device(s) added ==", new_count);
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
    identity: BeaconIdentity,
    port: u16,
    secs: u64,
    running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    let interfaces = get_local_interfaces();
    thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            announce_beacon(&identity, port, secs, &interfaces, Some(&running));
            std::thread::sleep(Duration::from_secs(2));
        }
    })
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
        eprintln!(
            "  {} thread did not stop within {}s",
            name,
            timeout.as_secs()
        );
    }
}
