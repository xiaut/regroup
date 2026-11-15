//! CLI module for regroup
//!
//! This module handles command-line interface functionality.

use std::env;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{root_config_dir, CONFIG_DIR, GIT_DIR, DEVICES_DIR, DEVICE_FILE, Device, load_config};
use crate::discovery::beacon::BeaconDevice;
use crate::discovery::{host_name, local_user};
use crate::git::{self, sync::git_add_and_commit_file};
use crate::ssh::config::{remove_block, update_block};
use crate::ssh::public_key::{self};

pub mod key_management;
pub mod receive_install_key;
pub mod receive_remove_key;

// ============================================================
// Error handling
// ============================================================

/// Terminate with an error message
pub fn die(msg: &str) -> ! {
    // Restore the terminal first: process::exit runs no destructors.
    crate::ui::restore();
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

/// Print a warning message
pub fn warn(msg: &str) {
    crate::ui::log(&format!("WARNING: {}", msg));
}

// ============================================================
// User interaction
// ============================================================

// ============================================================
// Selftest
// ============================================================

/// Run the built-in selftest: checks that only make sense against the real
/// environment (external tools, ssh-keygen fingerprint agreement).
/// Pure logic (config parsing, beacon signing) is covered by `cargo test`.
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

    // 2. host key fingerprint verification helper
    // Generate a throwaway key, then check that the fingerprint computed by
    // compute_fingerprint_from_public_key matches ssh-keygen -lf's output.
    // (Config parsing and beacon signing are covered by `cargo test`.)
    {
        println!("Generating test SSH key for fingerprint verification...");
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
        println!(
            "host key fingerprint — {}",
            if fingerprint_ok { "OK" } else { "FAIL" }
        );
        ok &= fingerprint_ok;

        let _ = fs::remove_file(&key_path);
        let _ = fs::remove_file(&pub_path);
    }

    println!("selftest: {}", if ok { "OK" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}

// ============================================================
// Usage
// ============================================================

/// The full help text, shared by `regroup -h` and the interactive help
/// panel (key '?' in the full-screen ui, 'h' in plain mode).
pub fn usage_text() -> &'static str {
"Up / Right / Down - from the overview, focus a pane: Devices (Up/Right, one list:\n                    localhost, added, detected rows) or Logs (Down)\n  Up/Down         - in the focused Devices/Users pane: select a row\n  Right/Enter     - in the Devices pane with the local or an added device\n                    selected, open its Users panel (device+user actions\n                    need a selected user); not offered when the device\n                    has no users — the Users panel is not reachable then\n  Left / Esc      - back (Users -> Devices -> overview)\n  Down/Enter      - in the focused Logs pane: fullscreen logs (Esc leaves)\n  a / n           - (detected row selected) add / ignore the device\n                    (ignored beacons are skipped for this run). Adding chains:\n                    the add-user prompt opens, then one remote call on the\n                    device (receive-add-local XOR receive-install-key),\n                    then a device-scoped sync\n  a / r           - (localhost row) add / remove the local device to/from\n                    the shared configuration (recoverable, unlike delete)\n  d               - (added row selected) delete the selected device\n  a               - (added row selected) add a user for the device (device-\n                    specific: no user needs to be selected). The user is\n                    temporary (tmp/) until confirmed — by a git sync, or by\n                    a successful key install. After the name is entered,\n                    regroup asks 'install key?': y runs the remote\n                    receive-install-key, n the remote receive-add-local\n                    (XOR), then a device-scoped sync runs\n  d               - (Users) set the selected user as the device's default user\n  i / r           - (Users, added device) install / remove an SSH key to\n                    user@device — works on a temporary user too, and a\n                    successful install confirms it (the outgoing connection\n                    is recorded as a connection file in your connections/)\n  r               - (localhost Users, 'external' row) revoke an incoming key:\n                    remove it from your authorized_keys and delete the\n                    connection file, locally (the '── external ──' group lists\n                    the incoming connections targeting the current user)\n  x               - (Users, added device) remove the selected user (tmp/ entry,\n                    or declared user file — committed so it propagates)\n  s               - sync git (pull then push), scoped where you press it:\n                    main menu: every device, every user; on a selected\n                    device row: every user of that device; on a selected\n                    user of an added device: every device, but that user\n                    on the selected device (a password may be asked for\n                    each user@device) — not offered on the local device\n  w               - (main menu) wipe the whole configuration and quit (y confirms, n cancels)\n  c               - (main menu) install the 'regroup-cd' shell function (y confirms, n cancels)\n  Enter/y         - confirm (delete, wipe, rename); in plain mode, add the device\n                    at the front of the queue\n  n               - cancel a confirmation\n  PgUp/PgDn       - scroll the activity log\n  ?               - show this help panel (Esc closes it)\n  q               - quit (from the main menu)\n\nConfiguration layout (~/.regroup-config/):\n  settings.json                     (local only: udp_port, wait_secs — never synced)\n  git/                              (source of truth, synced with peers)\n    devices/\n      <hostname>/                   (one directory per device)\n        device.json                 (latest known state: name, ssh_public_key, port)\n        users/<user>.json           (known users of the device, written by the device itself)\n        connections/<conn>.json     (connections of the device, owner-scoped)\n  tmp/                              (temporary overlay — never committed or synced)\n\nExit codes:\n  0  normal exit\n  1  selftest failure\n  2  configuration or environment error"}

/// Print usage information
pub fn usage() {
    println!("{}", usage_text());
}

// ============================================================
// Setup CD command
// ============================================================

/// The shell config file the 'regroup-cd' function is installed into:
/// ~/.zshrc for zsh, ~/.bashrc otherwise.
pub fn shell_rc_file(home: &str) -> PathBuf {
    // Detect shell from SHELL environment variable
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    if shell.contains("zsh") {
        PathBuf::from(home).join(".zshrc")
    } else {
        PathBuf::from(home).join(".bashrc")
    }
}

/// Install (or refresh) the regroup-cd shell function in the user's shell
/// config. An existing definition is replaced. Returns the shell config
/// file that was written.
pub fn setup_cd(home: &str) -> Result<PathBuf, String> {
    let config_dir = PathBuf::from(home).join(CONFIG_DIR);
    let function_def = format!("regroup-cd() {{ cd {}; }}", config_dir.to_string_lossy());

    let config_file = shell_rc_file(home);

    // Read current content
    let current_content = fs::read_to_string(&config_file).unwrap_or_default();

    // Prepare new content - remove any existing definition
    let new_content: String = current_content
        .lines()
        .filter(|line| !line.trim().starts_with("regroup-cd()"))
        .collect::<Vec<_>>()
        .join("\n");

    // Find the line with "case $- in" (non-interactive shell check)
    // We want to insert BEFORE this block so the function is available even when
    // the shell returns early for non-interactive sessions
    let case_line_index = new_content
        .lines()
        .position(|line| line.contains("case $- in"));

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
    fs::write(&config_file, final_content)
        .map(|_| config_file.clone())
        .map_err(|e| {
            format!(
                "cannot write to ~/{}: {}",
                config_file
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                e
            )
        })
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
        if !words.is_empty() && words[0] == "Host" && words[1..].contains(&name) {
            return true;
        }
    }
    false
}

/// Path of the config git repository for the given home:
/// `<home>/.regroup-config/git/`.
fn config_dir_of(home: &str) -> PathBuf {
    root_config_dir(home).join(GIT_DIR)
}

/// Check whether a device name is already used by another device.
///
/// A name is taken when it is already used by:
/// - a device in the regroup config (loaded from `devices/<name>/`),
/// - an existing `devices/<name>/device.json` file (even one that failed
///   to load), or
/// - an existing "Host <name>" block in ~/.ssh/config, whether regroup-managed
///   or configured by the user for an unrelated host.
pub fn device_name_taken(name: &str, home: &str, ssh_config: &Path) -> bool {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let root = root_config_dir(home);
    name_taken(
        name,
        &ssh_content,
        &root,
        &load_config().devices,
    )
}

/// Get detailed information about where a device name is already taken.
///
/// Returns a vector of conflicts, each indicating where the name is used:
/// - RegroupDevice: the name is used by a device in regroup config
/// - DeviceFileOnDisk: a device file exists on disk for this name
/// - SshConfigBlock: the name is in an SSH config Host block
pub fn find_name_conflicts(name: &str, home: &str, ssh_config: &Path) -> Vec<NameConflict> {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let root = root_config_dir(home);
    find_name_conflict(
        name,
        &ssh_content,
        &root,
        &load_config().devices,
    )
}

/// Information about where a device name is already taken
#[derive(Debug, Clone)]
pub enum NameConflict {
    /// The name is used by a device in the regroup configuration
    RegroupDevice { device_name: String },
    /// The name has a device file on disk (even if it failed to load)
    DeviceFileOnDisk,
    /// The name is in an existing SSH config Host block
    SshConfigBlock,
}

/// Check where a device name is already taken
fn find_name_conflict(
    name: &str,
    ssh_content: &str,
    root: &Path,
    known_devices: &[Device],
) -> Vec<NameConflict> {
    let mut conflicts = Vec::new();

    // Check if name is used by a known device in regroup config
    if let Some(device) = known_devices.iter().find(|d| d.name.as_str() == name) {
        conflicts.push(NameConflict::RegroupDevice {
            device_name: device.name.as_str().to_string(),
        });
    }

    // Check if a device file exists on disk, in either proxy layer (a
    // tmp/ placeholder counts too)
    if crate::config::proxy::device_file(root, name).is_some() {
        conflicts.push(NameConflict::DeviceFileOnDisk);
    }

    // Check if name is in SSH config
    if ssh_config_has_host(ssh_content, name) {
        conflicts.push(NameConflict::SshConfigBlock);
    }

    conflicts
}

fn name_taken(
    name: &str,
    ssh_content: &str,
    root: &Path,
    known_devices: &[Device],
) -> bool {
    !find_name_conflict(name, ssh_content, root, known_devices).is_empty()
}

/// Suggest the first free name for a device: `desired` if free, otherwise
/// "name-2", "name-3", ... Used as the default answer when asking the user
/// to pick a new name for a device whose name is already taken.
pub fn unique_device_name(desired: &str, home: &str, ssh_config: &Path) -> String {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let known_devices = load_config().devices;
    let root = root_config_dir(home);
    if !name_taken(desired, &ssh_content, &root, &known_devices) {
        return desired.to_string();
    }
    let mut n = 2;
    while n < 1000
        && name_taken(
            &format!("{}-{}", desired, n),
            &ssh_content,
            &root,
            &known_devices,
        )
    {
        n += 1;
    }
    format!("{}-{}", desired, n)
}

/// Add an accepted device to the configuration by adding its git remote
/// and pulling its authoritative device.json, then updating ~/.ssh/config
/// and known_hosts.
///
/// NEW FLOW (no cross-writing):
/// - Add git remote for the device
/// - Pull from the remote to get the device's self-written device.json
/// - Verify host key (security)
/// - Update SSH config
/// - The device file must be written by the device itself
///
/// This ensures write ownership isolation and eliminates race conditions
/// when two devices discover each other simultaneously.
///
/// The caller is responsible for the name: interactive mode asks the user to
/// pick a collision-free name first (see `device_name_taken`).
///
/// Returns the device from the pulled device.json (or from beacon data as fallback),
/// or None if the device could not be added.
pub fn add_device_to_config(
    home: &str,
    ssh_config: &Path,
    device: &BeaconDevice,
    name: &str,
    new_count: &mut usize,
) -> Option<Device> {
    let root = root_config_dir(home);

    // The beacon carries no user (users are configuration, not discovery):
    // nothing is known about the users of this device yet. The git remote
    // and the first pull therefore happen later, once a user is known —
    // either a temporary one added from the Users panel, or a confirmed one
    // pulled by a sync (see sync_git_remotes).

    // SECURITY: verify host key BEFORE adding anything
    let host_key_ok = match public_key::compute_fingerprint_from_public_key(device.ssh_public_key.as_str()) {
        Some(expected) => {
            match public_key::verify_and_add_to_known_hosts(&device.ip, device.port, &expected) {
                public_key::HostKeyVerification::Success => {
                    crate::ui::log(&format!("  - Added {} to ~/.ssh/known_hosts (host key verified)", name));
                    true
                }
                public_key::HostKeyVerification::HostUnreachable => {
                    warn(&format!("{} unreachable at {} — host key not verified, ssh will ask on first connection", name, device.ip));
                    true
                }
                public_key::HostKeyVerification::WriteError => {
                    warn(&format!("could not write {} to ~/.ssh/known_hosts", name));
                    true
                }
                public_key::HostKeyVerification::FingerprintMismatch { expected, actual } => {
                    crate::ui::log(&format!(
                        "[!] SECURITY: {} announces key {} at {}\n    but the server there serves a DIFFERENT host key — possible spoofing or MITM.\n    expected fingerprint: {}\n    actual fingerprint:   {}\n    Device NOT added.",
                        name,
                        &device.ssh_public_key.as_str()[..16.min(device.ssh_public_key.as_str().len())],
                        device.ip,
                        expected,
                        actual
                    ));
                    false
                }
            }
        }
        None => {
            crate::ui::log(&format!("[!] SECURITY: {} announces a host key that is not a valid OpenSSH public key — device NOT added.", name));
            false
        }
    };

    if !host_key_ok {
        return None;
    }

    // Try to load an existing device.json through the proxy (from a previous
    // pull in git/, or a tmp/ placeholder from a previous run)
    if let Some(device_path) = crate::config::proxy::device_file(&root, name) {
        if let Ok(existing_device) = crate::config::device::load_device_file(&device_path) {
            if existing_device.ssh_public_key == device.ssh_public_key {
                if update_ssh_config(ssh_config, name, &device.ip, &existing_device) {
                    *new_count += 1;
                    return Some(existing_device);
                }
            } else {
                warn(&format!("existing device.json for {} has different SSH key than beacon", name));
            }
        }
    }

    // Fallback: use beacon data
    // Note: We couldn't pull the device's authoritative device.json, so we create
    // a placeholder from the beacon data. This will be replaced when the device
    // pushes its own device.json and we pull it.
    let placeholder_device = Device {
        name: crate::config::DeviceName::from_str(name)
            .expect("name must be non-empty as checked by device_name_taken"),
        ssh_public_key: device.ssh_public_key.clone(),
        signing_public_key: device.signing_public_key.clone(),
        port: device.port,
        port_explicit: true,
        default_user: None,
    };

    // Save the placeholder into the tmp/ overlay so the device persists
    // across restarts. It is never committed and never synced: it stays out
    // of git/ and is purged automatically once the device's authoritative
    // device.json arrives in git/ through a sync.
    if let Err(e) = crate::config::proxy::write_tmp_device_file(&root, name, &placeholder_device) {
        die(&format!(
            "cannot save the placeholder device config for {name} in {}: {e}",
            root.join(crate::config::TMP_DIR).display()
        ));
    }

    if update_ssh_config(ssh_config, name, &device.ip, &placeholder_device) {
        crate::ui::log(&format!(
            "  - Added {} with placeholder data — will update from device on next sync",
            name
        ));
        crate::ui::log(&format!(
            "  - No user known on {name} yet — the add-user prompt opens next in the interactive flow (one remote call, then a sync)",
        ));
        *new_count += 1;
        Some(placeholder_device)
    } else {
        None
    }
}

/// Helper to update SSH config and log success/failure
fn update_ssh_config(ssh_config: &Path, name: &str, ip: &str, device: &Device) -> bool {
    match update_block(ssh_config, name, ip, device.port, device.port_explicit) {
        Ok(_) => {
            crate::ui::log(&format!("  - Added {} -> {} (~/.ssh/config updated)", name, ip));
            true
        }
        Err(e) => {
            warn(&format!("could not update SSH config for {}: {}", name, e));
            false
        }
    }
}

/// Update a device's signing public key by pulling from the device
/// to get its latest authoritative device.json.
///
/// Used when the user accepts a new signing key (device reinstall).
/// Instead of directly writing the device file, we pull from the device's
/// remote to get its latest self-written device.json which should contain
/// the new signing key.
///
/// If pull fails (device unreachable), we fall back to writing the signing
/// key locally - this is the ONLY case where we write another device's file.
pub fn trust_new_signing_key(home: &str, device: &BeaconDevice) {
    let conf = load_config();
    let existing_device = conf
        .devices
        .iter()
        .find(|m| m.ssh_public_key == device.ssh_public_key)
        .cloned();
    let existing_device = match existing_device {
        Some(m) => m,
        None => {
            warn(&format!("cannot update signing key: device with SSH key {} not found", device.ssh_public_key.as_str()));
            return;
        }
    };

    let root = root_config_dir(home);
    let config_dir = config_dir_of(home);

    // Try to pull from the device's remote first
    match git::sync::git_pull_remote(&config_dir, existing_device.name.as_str()) {
        Ok(_) => {
            if let Some(device_path) = crate::config::proxy::device_file(&root, existing_device.name.as_str()) {
                match crate::config::device::load_device_file(&device_path) {
                    Ok(updated_device) => {
                        if updated_device.signing_public_key == device.signing_public_key {
                            crate::ui::log(&format!("  - Updated signing key for {} (pulled from device)", existing_device.name));
                            return;
                        }
                    }
                    Err(e) => {
                        warn(&format!("could not load device config for {}: {}", existing_device.name, e));
                    }
                }
            }
            // Pull succeeded but device.json doesn't have the expected signing key yet
            warn(&format!("pulled device.json for {} does not have the expected signing key", existing_device.name));
        }
        Err(e) => {
            // Pull failed - device might not be reachable
            warn(&format!("could not pull from {} to update signing key: {}", device.name, e));
        }
    }

    // Fallback: We couldn't pull the updated device.json with the new signing key.
    // The signing key will be updated naturally when the device's own device.json
    // (with the new key) is pulled in a future sync. No local write needed.
    //
    // If this is a reinstall scenario, the user should ensure the device has
    // committed its new device.json with the updated signing key, then sync.
    warn(&format!(
        "Could not verify new signing key for {} — device.json may not have been updated yet. \
         The signing key will be updated when the device pushes its new config.",
        device.name
    ))
}

// ============================================================
// Git operations
// ============================================================

/// The scope of a git sync, set by where in the TUI the sync action was
/// triggered: the main menu syncs everything, a selected device row only
/// that device, a selected user pins that user on the selected device.
/// Syncing per user@device matters because each one may prompt for its own
/// password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncScope {
    /// Every known device, every known user of each device.
    All,
    /// One device, every known user of it.
    Device(String),
    /// Every known device and every user, except `device`, synced as `user`
    /// only.
    User { device: String, user: String },
}

impl SyncScope {
    /// Human-readable scope, shown in the sync log line.
    pub fn describe(&self) -> String {
        match self {
            SyncScope::All => "every device, every user".to_string(),
            SyncScope::Device(name) => format!("device {name}, every user of it"),
            SyncScope::User { device, user } => {
                format!("every device and user, except {device} as {user} only")
            }
        }
    }
}

/// Sync the git configuration repository, scoped (see [`SyncScope`]):
/// configure remotes for all known devices (except self), then pull from
/// and push to each in-scope user@device — every user of every device in
/// scope, each reached through its own URL so it can prompt for its own
/// password. The per-target pull/push results are always logged.
pub fn sync_git(home: &str, temp_users: &HashMap<String, String>, scope: &SyncScope) {
    // The root holds the git repository in git/ and the tmp/ overlay
    let root = root_config_dir(home);
    // The git repository lives in ~/.regroup-config/git (not the root directory)
    let config_dir = root.join(GIT_DIR);

    if !git::is_git_repo(&config_dir) {
        warn(&format!("{} is not a git repository", config_dir.display()));
        return;
    }

    crate::ui::log(&format!("Syncing git ({})...", scope.describe()));
    crate::ui::log(
        "Note: If any remote repository requires authentication, you may be prompted for credentials."
    );

    // Configure remotes for all known devices before syncing: pull/push can
    // only reach devices that already have a remote.
    let remote_errors = configure_remotes(&root, temp_users);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }
    let remotes = git::remote::git_list_remotes(&config_dir);
    if remotes.is_empty() {
        crate::ui::log("  - sync: no git remotes configured");
    } else {
        crate::ui::log(&format!("  - sync: remotes: {}", remotes.join(", ")));
    }

    // The (device, user) pairs to sync, per the scope. Each pair is synced
    // through its own URL (git accepts a URL where a remote name is
    // expected), so every user@device is reached independently of the named
    // remotes — which stay configured for the primary user of each device.
    let devices = syncable_devices();
    let (pairs, target_errors) = sync_targets(&root, &devices, temp_users, scope);
    let targets: Vec<git::sync::SyncTarget> = pairs
        .iter()
        .map(|(device, user)| git::sync::SyncTarget {
            label: format!("{user}@{device}"),
            target: git::remote::remote_url(user, device),
        })
        .collect();
    if targets.is_empty() {
        crate::ui::log("  - sync: no user@device to sync with");
    }

    // Sync with the targets (pull then push)
    let sync_errors = git::sync_with_targets(&config_dir, &targets);
    for error in target_errors.iter().chain(&sync_errors) {
        warn(&format!("[sync] {}", error));
    }

    // Eager purge: a successful sync may have brought the authoritative
    // counterpart of tmp/ overlay files — drop the now-stale tmp/ copies.
    if sync_errors.is_empty() {
        let purged = crate::config::proxy::purge_tmp_overlaps(&root);
        if purged > 0 {
            crate::ui::log(&format!(
                "  - purged {purged} stale tmp/ file(s) now present in git/"
            ));
        }
    }

    // A pull may have brought device files for devices we did not know yet:
    // configure their remotes so the next sync reaches them too.
    let remote_errors = configure_remotes(&root, temp_users);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }

    crate::ui::log("Git sync complete.");
}

/// The (device, user) pairs a scoped sync must reach, plus an error line
/// for each in-scope device with no known user. "Every user" of a device is
/// its merged user list (declared in `users/` plus the `tmp/` overlay),
/// with the session's temporary user (`temp_users`) added when missing. A
/// `User` scope pins its device to the selected user only.
fn sync_targets(
    root: &Path,
    devices: &[Device],
    temp_users: &HashMap<String, String>,
    scope: &SyncScope,
) -> (Vec<(String, String)>, Vec<String>) {
    let mut pairs = Vec::new();
    let mut errors = Vec::new();
    for device in devices {
        let name = device.name.as_str();
        if let SyncScope::Device(scoped) = scope
            && scoped != name
        {
            continue;
        }
        let users: Vec<String> = match scope {
            SyncScope::User { device, user } if device == name => vec![user.clone()],
            _ => {
                let mut users: Vec<String> =
                    crate::config::users::load_device_users_merged(root, name)
                        .map(|known| known.iter().map(|u| u.user.name.clone()).collect())
                        .unwrap_or_default();
                if let Some(temp) = temp_users.get(name)
                    && !users.contains(temp)
                {
                    users.push(temp.clone());
                }
                users.sort();
                users.dedup();
                users
            }
        };
        if users.is_empty() {
            errors.push(format!(
                "no user known on {name} — add a temporary user (Users panel) to sync with it"
            ));
            continue;
        }
        pairs.extend(users.into_iter().map(|user| (name.to_string(), user)));
    }
    (pairs, errors)
}

// ============================================================
// Device removal
// ============================================================

/// Ask a yes/no question on the terminal. Used by the CLI subcommands only;
/// interactive mode asks through the TUI.
pub fn confirm(question: &str) -> bool {
    print!("{question} [y/N]: ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim().to_lowercase();
    answer == "y" || answer == "yes"
}

/// Remove the local traces of a device outside the config repository:
/// ~/.ssh/config block, known_hosts entries (best effort: last known IP
/// only) and its git remote. Does not touch the device file.
///
/// Used both when the user removes the device here, and when a git sync
/// reveals it was removed on another device.
pub fn remove_local_traces(
    home: &str,
    ssh_config: &Path,
    device: &Device,
    last_ip: Option<&str>,
) {
    match remove_block(ssh_config, device.name.as_str()) {
        Ok(true) => {
            crate::ui::log(&format!(
                "  - Removed {} block from ~/.ssh/config",
                device.name.as_str()
            ));
        }
        Ok(false) => {}
        Err(e) => warn(&format!(
            "could not remove {} block from ~/.ssh/config: {}",
            device.name, e
        )),
    }
    if let Some(ip) = last_ip {
        public_key::remove_known_host(ip);
    }
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    // A missing remote is normal (never configured, or removal propagated
    // from another device): only a failing removal is reported.
    if git::remote::git_list_remotes(&config_dir)
        .iter()
        .any(|r| r == device.name.as_str())
        && let Err(e) = git::remote::git_remove_remote(&config_dir, device.name.as_str())
    {
        warn(&format!(
            "could not remove git remote {}: {}",
            device.name.as_str(), e
        ));
    }
}

/// Remove a device entirely: delete its device directory (device file,
/// users and connections; the committed deletion propagates to other
/// devices on git sync), then remove its local traces. Returns true when
/// the device directory was deleted.
pub fn remove_device(
    home: &str,
    ssh_config: &Path,
    device: &Device,
    last_ip: Option<&str>,
) -> bool {
    let root = root_config_dir(home);
    let config_dir = config_dir_of(home);
    let dir = crate::config::device_dir(&config_dir, &device.name);
    let mut ok = true;
    // Delete in BOTH proxy layers: a stale tmp/ placeholder must not
    // resurrect the device on the next read.
    match crate::config::proxy::delete_device(&root, device.name.as_str()) {
        Ok(existed_in_git) => {
            // `git add` on the directory stages every deletion inside it;
            // the committed deletion propagates to peers on sync.
            if existed_in_git
                && let Err(e) =
                    git_add_and_commit_file(&config_dir, &dir, &format!("Remove device {}", device.name))
            {
                warn(&format!(
                    "could not commit removal of {}: {}",
                    device.name, e
                ));
            }
        }
        Err(e) => {
            warn(&format!(
                "could not delete device directory for {}: {}",
                device.name, e
            ));
            ok = false;
        }
    }
    remove_local_traces(home, ssh_config, device, last_ip);
    ok
}

/// Add the local device to the shared configuration: (re)create its device
/// file and the current user's file in the git layer and commit them, so a
/// git sync shares them with the other devices. This is one of the only
/// two ways the local device joins the shared configuration — the other is
/// key 'a' on the localhost row in the TUI; the load path never adds it
/// implicitly. The `regroup receive-add-local` remote call (run by the
/// user from the shell, or on a target device over SSH; every remote API
/// command starts with 'receive-') goes through here.
/// No ~/.ssh/config block and no known_hosts entry: the local device has
/// no such traces.
/// Returns true when the device file is present in the git layer
/// afterwards.
pub fn add_local_device_to_config(home: &str) -> bool {
    let root = root_config_dir(home);
    let config_dir = config_dir_of(home);
    let local_name = host_name();
    // Device file (also the repository's initial commit when the add
    // happens on a fresh, commit-less repository).
    if let Err(e) = crate::config::loading::create_device_config(&config_dir) {
        warn(&format!("could not create the local device config: {e}"));
        return false;
    }
    // Current user file, recorded once and committed (same as at config
    // load, see load_config).
    let current_user = local_user();
    if !current_user.is_empty() && current_user != "unknown" {
        match crate::config::users::ensure_local_user_file(&config_dir, &local_name, &current_user) {
            Ok(true) => {
                let user_path =
                    crate::config::users::user_file_path(&config_dir, &local_name, &current_user);
                if let Err(e) = git_add_and_commit_file(
                    &config_dir,
                    &user_path,
                    &format!("Add user {current_user} on {local_name}"),
                ) {
                    warn(&format!("could not commit user file: {e}"));
                }
            }
            Ok(false) => {}
            Err(e) => warn(&format!(
                "could not record user {current_user} on {local_name}: {e}"
            )),
        }
    }
    crate::config::proxy::in_git(&root, &format!("{DEVICES_DIR}/{local_name}/{DEVICE_FILE}"))
}

/// Remove the local device from the shared configuration (key 'd' on the
/// localhost row when it is added): delete its directory from the git
/// layer and commit the deletion, so it propagates to the other devices
/// on git sync. The tmp/ overlay (local-only data) and ~/.ssh/config /
/// known_hosts are untouched — the local device has no such traces.
/// Returns true when the device file is gone from the git layer
/// afterwards.
pub fn remove_local_device_from_config(home: &str) -> bool {
    let root = root_config_dir(home);
    let config_dir = config_dir_of(home);
    let local_name = host_name();
    let rel = format!("{DEVICES_DIR}/{local_name}");
    let dir = config_dir.join(&rel);
    if dir.exists() {
        if let Err(e) = fs::remove_dir_all(&dir) {
            warn(&format!("could not delete {}: {e}", dir.display()));
            return false;
        }
        // `git add` on the directory stages every deletion inside it.
        if let Err(e) =
            git_add_and_commit_file(&config_dir, &dir, &format!("Remove device {local_name}"))
        {
            warn(&format!("could not commit removal of {local_name}: {e}"));
        }
    }
    !crate::config::proxy::in_git(&root, &format!("{rel}/{DEVICE_FILE}"))
}

/// Delete the whole regroup configuration: the ~/.ssh/config blocks of all
/// known devices, then ~/.regroup-config itself.
///
/// known_hosts entries are left alone: past IPs are not recorded anywhere
/// (only the last known IP is, in memory, during interactive sessions) —
/// `regroup remove` and the interactive deletion clean them up while the
/// IP is still known.
pub fn wipe(home: &str, ssh_config: &Path, devices: &[Device]) {
    for device in devices {
        if let Err(e) = remove_block(ssh_config, device.name.as_str()) {
            warn(&format!(
                "could not remove {} block from ~/.ssh/config: {}",
                device.name.as_str(), e
            ));
        }
    }
    let root = PathBuf::from(home).join(CONFIG_DIR);
    if !root.exists() {
        crate::ui::log(&format!("Nothing to wipe: {} does not exist.", root.display()));
        return;
    }
    if let Err(e) = fs::remove_dir_all(&root) {
        die(&format!("cannot remove {}: {}", root.display(), e));
    }
    crate::ui::log(&format!(
        "Deleted {} ({} device block(s) removed from ~/.ssh/config).",
        root.display(),
        devices.len()
    ));
}

/// The SSH user used for a device's git remote URL (one per device, the
/// remote's "primary" user), resolved through the config proxy: a temporary
/// user added this run (Users panel, persisted at
/// `tmp/devices/<name>/users/<user>.json`) wins; otherwise the first user
/// the device declared in its configuration (`users/` in `git/`, arrived
/// through git sync); otherwise a leftover temporary user from a previous
/// run. Devices with no known user are skipped: their remote is created
/// once a user is known.
fn resolve_ssh_users(root: &Path, devices: &[Device], temp_users: &HashMap<String, String>) -> HashMap<String, String> {
    let mut users = HashMap::new();
    for device in devices {
        if let Some(temp) = temp_users.get(device.name.as_str()) {
            users.insert(device.name.as_str().to_string(), temp.clone());
        } else if let Ok(known) = crate::config::users::load_device_users_merged(root, device.name.as_str())
            && let Some(chosen) = known.iter().find(|u| !u.temporary).or(known.first())
        {
            users.insert(device.name.as_str().to_string(), chosen.user.name.clone());
        }
    }
    users
}

/// All known devices except the local one — the devices a sync can reach.
///
/// Self-detection uses both the hostname and the SSH public key: a stale
/// device file written under an old hostname (hostnames can change, e.g.
/// Termux) must not come back as a remote pointing at ourselves.
fn syncable_devices() -> Vec<Device> {
    let conf = load_config();
    let local_name = host_name();
    let local_ssh_public_key = public_key::get_ssh_server_public_key();
    conf.devices
        .into_iter()
        .filter(|m| {
            m.name.as_str() != local_name && local_ssh_public_key.as_ref() != Some(&m.ssh_public_key)
        })
        .collect()
}

fn configure_remotes(root: &Path, temp_users: &HashMap<String, String>) -> Vec<String> {
    let local_name = host_name();
    let devices = syncable_devices();
    let config_dir = root.join(GIT_DIR);
    let ssh_users = resolve_ssh_users(root, &devices, temp_users);
    git::sync_git_remotes(&config_dir, &devices, &local_name, &ssh_users)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_has_host_matches_exact_blocks() {
        let content = "Host laptop\n  HostName 1.2.3.4\n\nHost a b laptop2\n  User x\n\n# commented\nHostname decoy\n";
        assert!(ssh_config_has_host(content, "laptop"));
        // Multi-pattern Host lines count too
        assert!(ssh_config_has_host(content, "laptop2"));
        assert!(!ssh_config_has_host(content, "laptop3"));
        // HostName (lowercase, no space) must not be treated as a Host block
        assert!(!ssh_config_has_host(content, "decoy"));
    }

    #[test]
    fn name_taken_detects_all_three_sources() {
        let dir = std::env::temp_dir().join(format!("regroup-cli-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");

        let device = Device {
            name: crate::config::DeviceName::new("known".to_string()).unwrap(),
            ssh_public_key: crate::ssh::keys::SshPublicKey::from("ssh-rsa AAAA"),
            signing_public_key: crate::ssh::keys::SigningPublicKeyB64::from("MCowBQYDK2VwAyEA=="),
            port: 22,
            port_explicit: false,
            default_user: None,
        };
        let devices = vec![device];
        // `dir` plays the role of the root config dir: the device file lives
        // in the git/ layer, a placeholder in the tmp/ overlay.
        let on_disk = crate::config::device_file_path(&dir.join(GIT_DIR), "on-disk");
        fs::create_dir_all(on_disk.parent().unwrap()).unwrap();
        fs::write(&on_disk, "{}").unwrap();
        let placeholder = crate::config::proxy::tmp_device_file(&dir, "tmp-only");
        fs::create_dir_all(placeholder.parent().unwrap()).unwrap();
        fs::write(&placeholder, "{}").unwrap();
        let ssh = "Host in-ssh\n  User x\n";

        assert!(name_taken("known", "", &dir, &devices), "device list");
        assert!(name_taken("on-disk", "", &dir, &devices), "device file in git/ layer");
        assert!(name_taken("tmp-only", "", &dir, &devices), "placeholder in tmp/ layer");
        assert!(name_taken("in-ssh", ssh, &dir, &devices), "ssh config block");
        assert!(!name_taken("free", ssh, &dir, &devices), "free name");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_targets_are_scoped_and_cover_every_user_of_a_device() {
        let dir = std::env::temp_dir().join(format!("regroup-sync-targets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        let root = dir; // plays the role of ~/.regroup-config
        let git = root.join(GIT_DIR);
        // laptop: alice declared in the git layer, bob temporary in tmp/.
        crate::config::users::ensure_local_user_file(&git, "laptop", "alice").unwrap();
        crate::config::proxy::write_tmp_user_file(&root, "laptop", "bob").unwrap();
        // phone: no user known at all.

        let mk = |name: &str| Device {
            name: crate::config::DeviceName::new(name.to_string()).unwrap(),
            ssh_public_key: crate::ssh::keys::SshPublicKey::from("ssh-rsa AAAA"),
            signing_public_key: crate::ssh::keys::SigningPublicKeyB64::from("MCowBQYDK2VwAyEA=="),
            port: 22,
            port_explicit: false,
            default_user: None,
        };
        let devices = vec![mk("laptop"), mk("phone")];
        let temp_users: HashMap<String, String> =
            HashMap::from([("laptop".to_string(), "carol".to_string())]);

        // All: every device, every user (declared + temporary + session temp user).
        let (pairs, errors) = sync_targets(&root, &devices, &temp_users, &SyncScope::All);
        assert_eq!(
            pairs,
            vec![
                ("laptop".to_string(), "alice".to_string()),
                ("laptop".to_string(), "bob".to_string()),
                ("laptop".to_string(), "carol".to_string()),
            ]
        );
        assert_eq!(errors.len(), 1, "phone has no user: one error line");
        assert!(errors[0].contains("phone"));

        // Device scope: only that device, still every user of it.
        let (pairs, errors) =
            sync_targets(&root, &devices, &temp_users, &SyncScope::Device("laptop".to_string()));
        assert_eq!(pairs.len(), 3);
        assert!(pairs.iter().all(|(d, _)| d == "laptop"));
        assert!(errors.is_empty());

        // Device scope on a device with no known user: no pair, one error.
        let (pairs, errors) =
            sync_targets(&root, &devices, &temp_users, &SyncScope::Device("phone".to_string()));
        assert!(pairs.is_empty());
        assert_eq!(errors.len(), 1);

        // User scope: every device, but the pinned device as that user only.
        let (pairs, errors) = sync_targets(
            &root,
            &devices,
            &temp_users,
            &SyncScope::User {
                device: "laptop".to_string(),
                user: "bob".to_string(),
            },
        );
        assert_eq!(pairs, vec![("laptop".to_string(), "bob".to_string())]);
        assert_eq!(errors.len(), 1, "phone still reported");

        let _ = fs::remove_dir_all(&root);
    }
}
