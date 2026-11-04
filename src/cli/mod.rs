//! CLI module for regroup
//!
//! This module handles command-line interface functionality.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::device::save_device_file;
use crate::config::{CONFIG_DIR, GIT_DIR, Machine, load_config};
use crate::discovery::beacon::BeaconDevice;
use crate::discovery::host_name;
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
    "regroup — find machines by their SSH host key and update ~/.ssh/config.\nDevices announce themselves via signed UDP beacons; regroup listens for them\ncontinuously and prompts for each new device.\n\nConfiguration is stored in ~/.regroup-config/git/ (a git repository) where\neach device owns a directory devices/<name>/ holding its latest known\nstate (device.json) and its lists (known users, connections). Git remotes\ncan be configured for all other devices to enable decentralized sync\n(press 's' in interactive mode to pull from and push to them).\n\nUsage:\n  ./regroup                continuous interactive mode, update ~/.ssh/config\n  ./regroup test           dry run: nothing is modified\n  ./regroup remove <name>  remove a device (device dir, ~/.ssh/config block, git remote)\n  ./regroup wipe [--yes]   delete the entire regroup configuration\n  ./regroup -v             verbose (toggle at runtime with 'v' in interactive mode)\n  ./regroup selftest       built-in checks (external tools, ssh-keygen fingerprint)\n  ./regroup setup-cd       install 'regroup-cd' shell function to change to config directory\n  ./regroup -h             this help\n\nInteractive commands (full-screen mode):\n  Up/Dn           - select a machine in the Detected panel\n  Enter/y         - add the selected machine (it moves to the Added panel)\n  n               - ignore the selected machine (its beacons are ignored this run)\n  s               - sync git (pull from remotes, push to remotes)\n  d               - delete a known device (select in the Added panel, Enter confirms, Esc cancels)\n  w               - wipe the whole configuration (Enter confirms, Esc cancels)\n  i               - install SSH key to a machine (first known machine)\n  r               - remove SSH key from a machine (first outgoing connection)\n  c               - install the 'regroup-cd' shell function (Enter confirms, Esc cancels)\n  v               - toggle verbose mode (this machine's identity, git sync details in the activity log)\n  PgUp/PgDn       - scroll the activity log\n  ?               - show/hide this help panel\n  q               - quit the program\n\nConfig repository structure (~/.regroup-config/git/):\n  settings.json                     (optional: udp_port, wait_secs)\n  devices/\n    <hostname>/                     (one directory per device)\n      device.json                  (latest known state: name, ssh_public_key, user, port)\n      users/<user>.json             (known users of the device, written by the device itself)\n      connections/<conn>.json       (connections of the device, owner-scoped)\n\nExit codes:\n  0  normal exit\n  1  selftest failure\n  2  configuration or environment error"
}

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
    PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR)
}

/// Check whether a device name is already used by another machine.
///
/// A name is taken when it is already used by:
/// - a device in the regroup config (loaded from `devices/<name>/`),
/// - an existing `devices/<name>/device.json` file (even one that failed
///   to load), or
/// - an existing "Host <name>" block in ~/.ssh/config, whether regroup-managed
///   or configured by the user for an unrelated host.
pub fn device_name_taken(name: &str, home: &str, ssh_config: &Path) -> bool {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    name_taken(
        name,
        &ssh_content,
        &config_dir,
        &load_config().machines,
    )
}

fn name_taken(
    name: &str,
    ssh_content: &str,
    config_dir: &Path,
    known_machines: &[Machine],
) -> bool {
    known_machines.iter().any(|m| m.name == name)
        || crate::config::device_file_path(config_dir, name).exists()
        || ssh_config_has_host(ssh_content, name)
}

/// Suggest the first free name for a device: `desired` if free, otherwise
/// "name-2", "name-3", ... Used as the default answer when asking the user
/// to pick a new name for a device whose name is already taken.
pub fn unique_device_name(desired: &str, home: &str, ssh_config: &Path) -> String {
    let ssh_content = fs::read_to_string(ssh_config).unwrap_or_default();
    let known_machines = load_config().machines;
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    if !name_taken(desired, &ssh_content, &config_dir, &known_machines) {
        return desired.to_string();
    }
    let mut n = 2;
    while n < 1000
        && name_taken(
            &format!("{}-{}", desired, n),
            &ssh_content,
            &config_dir,
            &known_machines,
        )
    {
        n += 1;
    }
    format!("{}-{}", desired, n)
}

/// Add an accepted device to the configuration (device file, ~/.ssh/config,
/// known_hosts) and commit it to the config git repository, under the given
/// name.
///
/// The device file is written in the accepted device's directory
/// (`devices/<name>/device.json`) and holds our latest known state of it —
/// the device itself writes its own file too, and git sync resolves
/// diverging writes last-writer-wins (most recent commit wins). The
/// per-device write isolation applies strictly to `users/` and
/// `connections/`: those are only ever written by the device itself.
///
/// The caller is responsible for the name: interactive mode asks the user to
/// pick a collision-free name first (see `device_name_taken`); an unavailable
/// name would overwrite an existing device file or ~/.ssh/config block.
///
/// Returns the machine as actually saved, or None if the device file could
/// not be written.
pub fn add_device_to_config(
    home: &str,
    ssh_config: &Path,
    device: &BeaconDevice,
    name: &str,
    new_count: &mut usize,
) -> Option<Machine> {
    let config_dir = config_dir_of(home);

    // Ensure the device's own directory exists
    if let Err(e) = fs::create_dir_all(crate::config::device_dir(&config_dir, name)) {
        warn(&format!("could not create device directory: {}", e));
        return None;
    }

    let device_path = crate::config::device_file_path(&config_dir, name);
    // The beacon explicitly carries the device's SSH port: mark it explicit so
    // ~/.ssh/config and the device file keep it (e.g. Termux port 8022).
    // Pin the device's Ed25519 signing key: future beacons must be signed
    // with this same key or the user is asked to confirm the change.
    let machine = Machine {
        name: name.to_string(),
        ssh_public_key: device.ssh_public_key.clone(),
        signing_public_key: device.signing_public_key.clone(),
        user: device.user.clone(),
        port: device.port,
        port_explicit: true,
    };

    // SECURITY: verify that the host key announced in the beacon is really
    // served by the SSH server at the announced ip:port, BEFORE writing
    // anything. A mismatch means a forged beacon or a MITM: refuse the device.
    match public_key::compute_fingerprint_from_public_key(device.ssh_public_key.as_str()) {
        Some(expected) => {
            match public_key::verify_and_add_to_known_hosts(&device.ip, device.port, &expected) {
                public_key::HostKeyVerification::Success => {
                    crate::ui::log(&format!(
                        "  + Added {} to ~/.ssh/known_hosts (host key verified)",
                        name
                    ));
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
                    crate::ui::log(&format!(
                        "[!] SECURITY: {} announces key {} at {}\n    but the server there serves a DIFFERENT host key — possible spoofing or MITM.\n    expected fingerprint: {}\n    actual fingerprint:   {}\n    Device NOT added.",
                        name,
                        &device.ssh_public_key.as_str()
                            [..16.min(device.ssh_public_key.as_str().len())],
                        device.ip,
                        expected,
                        actual
                    ));
                    return None;
                }
            }
        }
        None => {
            crate::ui::log(&format!(
                "[!] SECURITY: {} announces a host key that is not a valid OpenSSH public key — device NOT added.",
                name
            ));
            return None;
        }
    }

    match save_device_file(&device_path, &machine) {
        Ok(_) => {
            // Update SSH config
            if let Err(e) = update_block(
                ssh_config,
                name,
                &device.ip,
                &machine.user,
                machine.port,
                machine.port_explicit,
            ) {
                warn(&format!("could not update SSH config for {}: {}", name, e));
            } else {
                crate::ui::log(&format!(
                    "  + Added {} -> {} (~/.ssh/config updated)",
                    name, device.ip
                ));
            }

            // Commit to git so changes persist across runs
            if let Err(e) =
                git_add_and_commit_file(&config_dir, &device_path, &format!("Add device {}", name))
            {
                warn(&format!(
                    "could not commit device config for {}: {}",
                    name, e
                ));
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

    let config_dir = config_dir_of(home);
    let device_path = crate::config::device_file_path(&config_dir, &machine.name);
    if let Err(e) = save_device_file(&device_path, &machine) {
        warn(&format!(
            "could not update signing key for {}: {}",
            machine.name, e
        ));
        return;
    }
    if let Err(e) = git_add_and_commit_file(
        &config_dir,
        &device_path,
        &format!("Update signing key for {}", machine.name),
    ) {
        warn(&format!(
            "could not commit signing key update for {}: {}",
            machine.name, e
        ));
    }
}

// ============================================================
// Git operations
// ============================================================

/// Sync the git configuration repository: configure remotes for all known
/// devices (except self), then pull from all remotes and push to all remotes.
/// In verbose mode, the per-remote pull/push results are logged.
pub fn sync_git(home: &str, verbose: bool) {
    // The git repository lives in ~/.regroup-config/git (not the root directory)
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);

    if !git::is_git_repo(&config_dir) {
        warn(&format!("{} is not a git repository", config_dir.display()));
        return;
    }

    crate::ui::log("Syncing git...");

    // Configure remotes for all known machines before syncing: pull/push can
    // only reach devices that already have a remote.
    let remote_errors = configure_remotes(&config_dir);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }
    if verbose {
        let remotes = git::remote::git_list_remotes(&config_dir);
        if remotes.is_empty() {
            crate::ui::log("  . sync: no git remotes configured");
        } else {
            crate::ui::log(&format!("  . sync: remotes: {}", remotes.join(", ")));
        }
    }

    // Sync with remotes (pull then push)
    let sync_errors = git::sync_with_remotes(&config_dir, verbose);
    for error in &sync_errors {
        warn(&format!("[sync] {}", error));
    }

    // A pull may have brought device files for machines we did not know yet:
    // configure their remotes so the next sync reaches them too.
    let remote_errors = configure_remotes(&config_dir);
    for error in &remote_errors {
        warn(&format!("[sync] {}", error));
    }

    crate::ui::log("Git sync complete.");
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
/// reveals it was removed on another machine.
pub fn remove_local_traces(
    home: &str,
    ssh_config: &Path,
    machine: &Machine,
    last_ip: Option<&str>,
) {
    match remove_block(ssh_config, &machine.name) {
        Ok(true) => {
            crate::ui::log(&format!(
                "  - Removed {} block from ~/.ssh/config",
                machine.name
            ));
        }
        Ok(false) => {}
        Err(e) => warn(&format!(
            "could not remove {} block from ~/.ssh/config: {}",
            machine.name, e
        )),
    }
    if let Some(ip) = last_ip {
        public_key::remove_known_host(ip);
    }
    let config_dir = PathBuf::from(home).join(CONFIG_DIR).join(GIT_DIR);
    // A missing remote is normal (never configured, or removal propagated
    // from another machine): only a failing removal is reported.
    if git::remote::git_list_remotes(&config_dir)
        .iter()
        .any(|r| r == &machine.name)
        && let Err(e) = git::remote::git_remove_remote(&config_dir, &machine.name)
    {
        warn(&format!(
            "could not remove git remote {}: {}",
            machine.name, e
        ));
    }
}

/// Remove a device entirely: delete its device directory (device file,
/// users and connections; the committed deletion propagates to other
/// machines on git sync), then remove its local traces. Returns true when
/// the device directory was deleted.
pub fn remove_device(
    home: &str,
    ssh_config: &Path,
    machine: &Machine,
    last_ip: Option<&str>,
) -> bool {
    let config_dir = config_dir_of(home);
    let dir = crate::config::device_dir(&config_dir, &machine.name);
    let mut ok = true;
    match fs::remove_dir_all(&dir) {
        Ok(()) => {
            // `git add` on the directory stages every deletion inside it.
            if let Err(e) =
                git_add_and_commit_file(&config_dir, &dir, &format!("Remove device {}", machine.name))
            {
                warn(&format!(
                    "could not commit removal of {}: {}",
                    machine.name, e
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            warn(&format!(
                "could not delete device directory for {}: {}",
                machine.name, e
            ));
            ok = false;
        }
    }
    remove_local_traces(home, ssh_config, machine, last_ip);
    ok
}

/// Delete the whole regroup configuration: the ~/.ssh/config blocks of all
/// known devices, then ~/.regroup-config itself.
///
/// known_hosts entries are left alone: past IPs are not recorded anywhere
/// (only the last known IP is, in memory, during interactive sessions) —
/// `regroup remove` and the interactive deletion clean them up while the
/// IP is still known.
pub fn wipe(home: &str, ssh_config: &Path, machines: &[Machine]) {
    for machine in machines {
        if let Err(e) = remove_block(ssh_config, &machine.name) {
            warn(&format!(
                "could not remove {} block from ~/.ssh/config: {}",
                machine.name, e
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
        machines.len()
    ));
}

/// Configure git remotes for all known machines except this device.
///
/// Self-detection uses both the hostname and the SSH public key: a stale
/// device file written under an old hostname (hostnames can change, e.g.
/// Termux) must not come back as a remote pointing at ourselves.
fn configure_remotes(config_dir: &Path) -> Vec<String> {
    let conf = load_config();
    let local_name = host_name();
    let local_ssh_public_key = public_key::get_ssh_server_public_key();
    let machines: Vec<Machine> = conf
        .machines
        .into_iter()
        .filter(|m| {
            m.name != local_name && local_ssh_public_key.as_ref() != Some(&m.ssh_public_key)
        })
        .collect();
    git::sync_git_remotes(config_dir, &machines, &local_name)
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

        let machine = Machine {
            name: "known".to_string(),
            ssh_public_key: crate::ssh::keys::SshPublicKey::from("ssh-rsa AAAA"),
            signing_public_key: crate::ssh::keys::SigningPublicKeyB64::from("MCowBQYDK2VwAyEA=="),
            user: String::new(),
            port: 22,
            port_explicit: false,
        };
        let machines = vec![machine];
        let on_disk = crate::config::device_file_path(&dir, "on-disk");
        fs::create_dir_all(on_disk.parent().unwrap()).unwrap();
        fs::write(&on_disk, "{}").unwrap();
        let ssh = "Host in-ssh\n  User x\n";

        assert!(name_taken("known", "", &dir, &machines), "machine list");
        assert!(name_taken("on-disk", "", &dir, &machines), "device file on disk");
        assert!(name_taken("in-ssh", ssh, &dir, &machines), "ssh config block");
        assert!(!name_taken("free", ssh, &dir, &machines), "free name");

        let _ = fs::remove_dir_all(&dir);
    }
}
