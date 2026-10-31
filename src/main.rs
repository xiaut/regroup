// regroup — find machines by their SSH host key fingerprint and update ~/.ssh/config.
//
// Configuration is stored in ~/.regroup-config/ git repository where each device
// has its own JSON file in the devices/ directory. Git remotes are automatically
// configured for all other devices to enable decentralized sync.
//
// Discovery and sync run in a loop until no new devices are found, handling IP
// address changes automatically. Each device owns its own config file and can only
// overwrite it. Conflicts on self-config are resolved by asking the user.
//
// Every run announces this machine ("name ip") over UDP broadcast AND listens
// for the other machines' announcements during the same window. Candidates are
// resolved by fingerprint (ssh-keyscan | ssh-keygen -lf -), so random hotspot
// subnets are no problem. A machine listed in the config that turns out to be
// THIS machine is detected as local and never written into its own ~/.ssh/config.
//
// Build (Termux):  pkg install rust openssh && cargo build --release
// Build (Ubuntu): sudo apt install cargo && cargo build --release
//
// Run:            ./regroup                (announce + listen + sync, update ~/.ssh/config)
//                 ./regroup test           (dry run: nothing is modified)
//                 ./regroup -v             (verbose)
//                 ./regroup -r             (refresh config from git remotes, then sync)
//                 ./regroup 192.168.43     (also scan that /24 once)
//                 ./regroup selftest       (built-in checks)
//                 ./regroup -h             (this help)
//
// Exit codes: 0 = all machines found; 1 = at least one missing;
//             2 = configuration or environment error; 3 = user declined conflict resolution.

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

use cli::{die, run_selftest, usage};
use config::{load_config, CONFIG_DIR};
use discovery::{host_name, my_ip};
use discovery::sync::discovery_sync_loop;
use git::{git_list_remotes, is_git_repo};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut help = false;
    let mut selftest = false;
    let mut net_arg: Option<String> = None;
    let mut refresh = false;
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
        } else if a == "-r" || a == "--refresh" {
            refresh = true;
        } else if a.contains('.') {
            net_arg = Some(a.clone());
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
    if verbose {
        println!(
            "  . this machine: {} / {}",
            local_name,
            local_ip.as_deref().unwrap_or("unknown IP")
        );
    }

    // --- Discovery and sync loop ---
    let (found, missing) = discovery_sync_loop(&conf, &ssh_config, &local_name, &local_ip, dry_run, verbose, refresh, &net_arg);

    // Check if this device is alone (only found itself, no remotes configured)
    if !dry_run && found == 1 && missing == 0 {
        let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
        if is_git_repo(&config_dir) {
            let remotes = git_list_remotes(&config_dir);
            if remotes.is_empty() {
                println!("\n=== Manual git sync setup ===");
                println!("This device is alone in the config. To sync with other devices:");
                println!("\nFrom this device (if you can SSH to others):");
                println!("  cd ~/.regroup-config");
                println!("  git remote add <hostname> user@hostname:~/.regroup-config");
                println!("  git push <hostname> main");
                println!("\nFrom another device (to pull this device's config):");
                println!("  cd ~/.regroup-config");
                println!("  git remote add {} {}:~/.regroup-config", local_name, local_name);
                println!("  git pull {} main", local_name);
                println!("\nNote: Authentication direction matters - you need SSH access in the direction you're syncing.");
            }
        }
    }

    println!("== {} found, {} missing ==", found, missing);
    std::process::exit(if missing == 0 { 0 } else { 1 });
}
