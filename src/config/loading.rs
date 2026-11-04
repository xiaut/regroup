//! Configuration loading module
//!
//! This module handles the main configuration loading logic.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::device::{load_device_file, save_device_file};
use super::settings::{load_global_settings, save_default_global_settings};
use super::users::ensure_local_user_file;
use super::{
    CONFIG_DIR, Conf, DEVICES_DIR, DEVICE_FILE, GLOBAL_CONFIG_FILE, GlobalSettings, Machine,
    device_dir, device_file_path,
};
use crate::discovery::local::host_name;
use crate::git::git_init;
use crate::ssh::keys::SigningPublicKeyB64;
use crate::ssh::public_key::get_ssh_server_public_key;

/// Track warnings that have already been displayed to avoid duplicates
static WARNING_TRACKER: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Terminate with an error message
fn die(msg: &str) -> ! {
    // Restore the terminal first: process::exit runs no destructors.
    crate::ui::restore();
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message only once per execution
fn warn(msg: &str) {
    let tracker = WARNING_TRACKER.get_or_init(|| Mutex::new(HashSet::new()));
    let mut tracker = tracker.lock().unwrap();
    if tracker.insert(msg.to_string()) {
        crate::ui::log(&format!("WARNING: {}", msg));
    }
}

// ============================================================
// Auto-create device configuration
// ============================================================

/// Auto-create device configuration with self-detected information
pub fn create_device_config(config_dir: &Path) -> Result<Machine, String> {
    let hostname = host_name();

    // Validate hostname
    let hostname_lower = hostname.to_lowercase();
    if hostname_lower == "localhost" || hostname_lower == "127.0.0.1" || hostname_lower == "local" {
        return Err(format!(
            "Hostname '{}' is not valid for device discovery. Please ensure the system has a proper hostname.",
            hostname
        ));
    }

    let ssh_public_key = get_ssh_server_public_key().ok_or_else(|| {
        "Could not detect SSH public key. Ensure SSH server is running on this machine.".to_string()
    })?;

    // Pin our own signing key in the device file: the machines that accept
    // this device verify our beacons against it.
    let signing_public_key = match crate::signing::initialize_signing_keys() {
        Ok((_signing_key, verifying_key)) => {
            use base64::Engine;
            SigningPublicKeyB64::from(
                base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes()),
            )
        }
        Err(e) => return Err(format!("cannot initialize signing keys: {}", e)),
    };

    // Determine the current user
    let current_user = env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());

    let machine = Machine {
        name: hostname.clone(),
        ssh_public_key,
        signing_public_key,
        user: current_user,
        port: 22,
        port_explicit: false,
    };

    // Validate the machine configuration
    super::validation::validate_machine(&machine)?;

    // Save to the device's own directory
    let dir = device_dir(config_dir, &hostname);
    if let Err(e) = fs::create_dir_all(&dir) {
        return Err(format!("cannot create device directory: {}", e));
    }

    let device_path = device_file_path(config_dir, &hostname);
    if let Err(e) = save_device_file(&device_path, &machine) {
        return Err(format!("cannot save device config: {}", e));
    }

    // First run: the device file is the repository's initial commit, so the
    // local branch exists before any sync with another device (a removal
    // on a commit-less repository cannot be committed).
    if let Err(e) = crate::git::sync::git_add_and_commit_file(
        config_dir,
        &device_path,
        &format!("Add device {hostname}"),
    ) {
        warn(&format!("could not commit device config: {}", e));
    }

    Ok(machine)
}

// ============================================================
// Main configuration loading
// ============================================================

/// Load the complete configuration
///
/// Configuration structure:
/// ~/.regroup-config/              - Root config directory (NOT a git repo)
///   └── git/                      - Git repository with shared config
///       ├── devices/<name>/       - One directory per device, written
///       │   ├── device.json       -   only by that device (see below)
///       │   ├── users/            -   known users of the device
///       │   └── connections/      -   connections to/from the device
///       └── settings.json         - Global settings
///
/// Storage rule: each device only ever writes inside its own directory;
/// other devices discover its files through git sync. On conflicts the
/// most recent commit wins (last-writer-wins).
pub fn load_config() -> Conf {
    // Root config directory
    let home = match env::var("HOME") {
        Ok(h) if !h.is_empty() => h,
        _ => die("HOME variable not set"),
    };
    let root_config_dir = PathBuf::from(&home).join(CONFIG_DIR);

    // Git config directory (this IS the git repo)
    let config_dir = root_config_dir.join(super::GIT_DIR);

    // Ensure root config directory exists
    if let Err(e) = fs::create_dir_all(&root_config_dir) {
        die(&format!(
            "cannot initialize config directory {}: {}",
            root_config_dir.display(),
            e
        ));
    }

    // Ensure git config directory exists and is a git repository
    if let Err(e) = fs::create_dir_all(&config_dir) {
        die(&format!(
            "cannot initialize git config directory {}: {}",
            config_dir.display(),
            e
        ));
    }

    // Initialize git repository in the git/ subdirectory
    if let Err(e) = git_init(&config_dir) {
        die(&format!(
            "cannot initialize git repository in {}: {}",
            config_dir.display(),
            e
        ));
    }

    // Create default settings if they don't exist
    if !config_dir.join(GLOBAL_CONFIG_FILE).exists()
        && let Err(e) = save_default_global_settings(&config_dir)
    {
        die(&format!("cannot create default settings: {}", e));
    }

    // Migrate the legacy flat layout (devices/<name>.json) to the
    // per-device directories, if any remains.
    migrate_legacy_devices(&config_dir);

    // Create devices directory
    let devices_dir = config_dir.join(DEVICES_DIR);
    if let Err(e) = fs::create_dir_all(&devices_dir) {
        die(&format!("cannot create devices directory: {}", e));
    }

    // Load global settings
    let GlobalSettings {
        udp_port,
        wait_secs,
    } = load_global_settings(&config_dir);

    // Load all device configurations: one device.json per device directory
    let mut machines: Vec<Machine> = Vec::new();

    if let Ok(entries) = fs::read_dir(&devices_dir) {
        for entry in entries.flatten() {
            let device_path = entry.path().join(DEVICE_FILE);
            if device_path.is_file() {
                match load_device_file(&device_path) {
                    Ok(machine) => machines.push(machine),
                    Err(e) => warn(&format!("skipping {}: {}", device_path.display(), e)),
                }
            }
        }
    }

    // Check if current machine's config exists, if not create it
    let local_hostname = host_name();
    let local_ssh_public_key = get_ssh_server_public_key();

    let current_machine_exists = machines.iter().any(|m| {
        m.name == local_hostname || (local_ssh_public_key.as_ref() == Some(&m.ssh_public_key))
    });

    if !current_machine_exists {
        // Auto-create this machine's configuration
        match create_device_config(&config_dir) {
            Ok(new_machine) => {
                machines.push(new_machine);
            }
            Err(e) => {
                warn(&format!("could not create machine config: {}", e));
            }
        }
    }

    // Record the current user on the local machine (first run only):
    // `devices/<self>/users/<user>.json` is written once and committed,
    // so other machines learn who this machine's users are through git.
    if machines.iter().any(|m| m.name == local_hostname) {
        let current_user =
            env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
        if !current_user.is_empty() {
            match ensure_local_user_file(&config_dir, &local_hostname, &current_user) {
                Ok(true) => {
                    let user_path = super::users::user_file_path(
                        &config_dir,
                        &local_hostname,
                        &current_user,
                    );
                    if let Err(e) = crate::git::sync::git_add_and_commit_file(
                        &config_dir,
                        &user_path,
                        &format!("Add user {current_user} on {local_hostname}"),
                    ) {
                        warn(&format!("could not commit user file: {}", e));
                    }
                }
                Ok(false) => {}
                Err(e) => warn(&format!(
                    "could not record user {current_user} on {local_hostname}: {}",
                    e
                )),
            }
        }
    }

    // Sort machines by name for consistent ordering
    machines.sort_by(|a, b| a.name.cmp(&b.name));

    if machines.is_empty() {
        warn(&format!(
            "no machines configured in {} — first machine will be auto-created on first run",
            config_dir.display()
        ));
    }

    Conf {
        udp_port,
        wait_secs,
        machines,
    }
}

/// One-time migration of the legacy layout to the per-machine layout:
/// move `devices/<name>.json` to `devices/<name>/device.json` and
/// commit both. Files already migrated are left alone; failures are
/// reported but never abort the load.
fn migrate_legacy_devices(config_dir: &Path) {
    let devices_dir = config_dir.join(DEVICES_DIR);
    if !devices_dir.exists() {
        return;
    }
    let Ok(entries) = fs::read_dir(&devices_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let legacy_path = entry.path();
        if legacy_path.extension().and_then(|e| e.to_str()) != Some("json") || !legacy_path.is_file()
        {
            continue;
        }
        let Some(name) = legacy_path.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        let new_path = device_file_path(config_dir, name);
        if new_path.exists() {
            // Already migrated: drop the stale copy so it stops haunting
            // older versions of regroup. If the removal fails, the stale
            // file stays and is simply ignored.
            if let Err(e) = fs::remove_file(&legacy_path) {
                warn(&format!(
                    "could not remove legacy device file {}: {}",
                    legacy_path.display(),
                    e
                ));
            }
            continue;
        }
        if let Err(e) = fs::create_dir_all(device_dir(config_dir, name)) {
            warn(&format!("could not create machine directory for {name}: {}", e));
            continue;
        }
        if let Err(e) = fs::rename(&legacy_path, &new_path) {
            warn(&format!(
                "could not migrate {} to {}: {}",
                legacy_path.display(),
                new_path.display(),
                e
            ));
            continue;
        }
        // Commit the new file, then the removal of the legacy file. The
        // removal commit is only attempted when the legacy file was
        // actually tracked in git (a never-committed file only existed on
        // this machine, nothing to propagate).
        if let Err(e) = crate::git::sync::git_add_and_commit_file(
            config_dir,
            &new_path,
            &format!("Migrate device {name} to devices/{name}/device.json"),
        ) {
            warn(&format!("could not commit migration of {name}: {}", e));
        }
        if crate::git::sync::git_is_tracked(config_dir, &legacy_path) {
            if let Err(e) = crate::git::sync::git_add_and_commit_file(
                config_dir,
                &legacy_path,
                &format!("Remove legacy device file for {name}"),
            ) {
                warn(&format!("could not commit migration of {name}: {}", e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// migrate_legacy_devices moves every devices/<name>.json to
    /// devices/<name>/device.json, does not touch already-migrated
    /// machines, and ignores non-JSON files.
    #[test]
    fn migrate_legacy_devices_moves_files() {
        let dir = std::env::temp_dir()
            .join(format!("regroup-migrate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let devices_dir = dir.join(DEVICES_DIR);
        fs::create_dir_all(&devices_dir).unwrap();
        fs::write(
            devices_dir.join("oldlaptop.json"),
            r#"{"name": "oldlaptop", "ssh_public_key": "ssh-rsa AAAA",
                "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
        )
        .unwrap();
        // Two users of different machines with the same legacy layout:
        // both files must move into their own machine directory.
        fs::write(
            devices_dir.join("oldphone.json"),
            r#"{"name": "oldphone", "ssh_public_key": "ssh-rsa BBBB",
                "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
        )
        .unwrap();
        fs::write(devices_dir.join("notes.txt"), "not json").unwrap();

        migrate_legacy_devices(&dir);

        assert!(device_file_path(&dir, "oldlaptop").is_file());
        assert!(device_file_path(&dir, "oldphone").is_file());
        assert!(!devices_dir.join("oldlaptop.json").exists());
        assert!(!devices_dir.join("oldphone.json").exists());
        assert!(devices_dir.join("notes.txt").exists());

        // A second run is a no-op, and a machine that already has its
        // device.json keeps its (possibly newer) content: the stale legacy
        // copy is dropped.
        fs::create_dir_all(dir.join(DEVICES_DIR).join("oldphone")).unwrap();
        fs::write(
            device_file_path(&dir, "oldphone"),
            r#"{"name": "oldphone", "ssh_public_key": "ssh-rsa NEWER",
                "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
        )
        .unwrap();
        fs::write(devices_dir.join("oldphone.json"), "stale copy").unwrap();
        migrate_legacy_devices(&dir);
        let kept = fs::read_to_string(device_file_path(&dir, "oldphone")).unwrap();
        assert!(kept.contains("NEWER"), "migrated device.json must win over stale copy");
        assert!(!devices_dir.join("oldphone.json").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
