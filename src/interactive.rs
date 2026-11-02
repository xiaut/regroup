//! Interactive mode: regroup's main loop.
//!
//! Listens for verified beacons (forwarded by the listener thread through a
//! channel) and for user input (forwarded by the stdin reader thread), and
//! manages the pending-device queue, the signing-key rotation prompt and the
//! rename prompt. All decisions that affect what is written to disk live here.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::cli::{
    add_device_to_config, device_name_taken, refresh_git, trust_new_signing_key, unique_device_name,
};
use crate::config::{Machine, load_config};
use crate::discovery::beacon::BeaconDevice;
use crate::ssh::config::update_block;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};
use crate::ssh::public_key;

/// Interval at which an unanswered question is re-displayed.
const PROMPT_REPEAT_INTERVAL: Duration = Duration::from_secs(2);

/// Per-run context shared by every branch of the interactive loop.
pub struct Context<'a> {
    pub home: &'a str,
    pub ssh_config: &'a Path,
    pub dry_run: bool,
    /// This machine's SSH server public key: its unique identifier.
    pub local_public_key: &'a SshPublicKey,
}

/// Run the interactive loop until the user quits or the listener thread dies.
///
/// `beacons` carries devices announced by other machines (already
/// signature-verified and age-checked by the listener). A stdin reader thread
/// is spawned here; each input line is one command (or one answer to a
/// question). Returns the number of devices accepted this run.
pub fn run(ctx: &Context, udp_port: u16, beacons: &Receiver<BeaconDevice>) -> usize {
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

    println!("\nListening for devices on UDP port {udp_port}...");
    println!("Press 'r' to refresh git, 'q' to quit.");
    println!("New devices will be prompted for confirmation.\n");

    let mut state = InteractiveState::new();
    loop {
        // Check for new beacons from the listener. A beacon that was fully
        // handled (filtered out, known device, rotation prompt raised) skips
        // input processing for this iteration, like the `continue` it replaces.
        match beacons.try_recv() {
            Ok(device) => {
                if state.handle_beacon(ctx, device) {
                    continue;
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }

        let user_command = cmd_rx.try_recv();
        if state.handle_input(ctx, user_command) {
            break;
        }

        // Small delay to prevent busy waiting
        thread::sleep(Duration::from_millis(100));
    }
    state.new_count
}

/// All mutable state of the interactive loop.
struct InteractiveState {
    /// SSH public keys of accepted devices.
    known_keys: HashSet<SshPublicKey>,
    /// Machine info by SSH public key for known devices.
    key_to_machine: HashMap<SshPublicKey, Machine>,
    /// Pinned Ed25519 signing keys (feature D): machines are keyed by their
    /// SSH public key, and their beacons must be signed with the signing key
    /// pinned at first acceptance. A different key means reinstall (ask the
    /// user) or spoofing (reject).
    pinned_signing_keys: HashMap<SshPublicKey, SigningPublicKeyB64>,
    /// Signing keys the user explicitly refused (rotation declined).
    rejected_signing_keys: HashSet<(SshPublicKey, SigningPublicKeyB64)>,
    /// Device currently prompting for a new signing key (reinstall or spoofing).
    rotation_pending: Option<BeaconDevice>,
    /// Queue for pending devices to confirm.
    pending_devices: VecDeque<BeaconDevice>,
    /// Pending devices by public key, to prevent duplicates.
    pending_keys: HashSet<SshPublicKey>,
    /// Last known IP for known devices, to avoid redundant notifications.
    known_ip: HashMap<SshPublicKey, String>,
    /// Devices the user rejected, to avoid re-asking.
    rejected_keys: HashSet<SshPublicKey>,
    /// Device waiting for the user to pick a new name, plus the suggested
    /// name. Set when the user accepts a device whose name is already taken;
    /// the next input line is then interpreted as the new name instead of a
    /// command. Only reachable in interactive mode: dry run never adds devices.
    rename_state: Option<(BeaconDevice, String)>,
    /// Devices accepted this run.
    new_count: usize,
    /// When the current question prompt was last displayed.
    last_prompt_time: Instant,
    /// Whether the idle "Waiting for devices" prompt is on screen.
    waiting_prompt_shown: bool,
}

impl InteractiveState {
    fn new() -> Self {
        let mut state = Self {
            known_keys: HashSet::new(),
            key_to_machine: HashMap::new(),
            pinned_signing_keys: HashMap::new(),
            rejected_signing_keys: HashSet::new(),
            rotation_pending: None,
            pending_devices: VecDeque::new(),
            pending_keys: HashSet::new(),
            known_ip: HashMap::new(),
            rejected_keys: HashSet::new(),
            rename_state: None,
            new_count: 0,
            last_prompt_time: Instant::now(),
            waiting_prompt_shown: false,
        };
        state.reload();
        state
    }

    /// Rebuild the lookup maps from the device files on disk (e.g. after a
    /// git refresh pulled new device files).
    fn reload(&mut self) {
        let conf = load_config();
        self.known_keys = conf
            .machines
            .iter()
            .map(|m| m.ssh_public_key.clone())
            .collect();
        self.key_to_machine = conf
            .machines
            .iter()
            .map(|m| (m.ssh_public_key.clone(), m.clone()))
            .collect();
        self.pinned_signing_keys = conf
            .machines
            .iter()
            .map(|m| (m.ssh_public_key.clone(), m.signing_public_key.clone()))
            .collect();
    }

    fn reset_prompt_time(&mut self) {
        self.last_prompt_time = Instant::now();
    }

    /// Whether the current question prompt should be re-displayed.
    fn prompt_due(&self, has_input: bool) -> bool {
        !has_input && self.last_prompt_time.elapsed() >= PROMPT_REPEAT_INTERVAL
    }

    /// Handle one announced device.
    ///
    /// Returns true when the beacon was fully handled and input processing
    /// must be skipped for this loop iteration (the `continue` of the
    /// original loop).
    fn handle_beacon(&mut self, ctx: &Context, device: BeaconDevice) -> bool {
        // IMPORTANT: Filter out self-detection using SSH public key only.
        // The SSH public key is the unique identifier for a machine.
        // We cannot use hostname (Termux changes it) or IP (network changes).
        if device.ssh_public_key == *ctx.local_public_key {
            return true;
        }

        // Signing key pinning: a beacon from a known machine must be
        // signed with the key pinned when it was accepted.
        if let Some(pinned) = self.pinned_signing_keys.get(&device.ssh_public_key)
            && pinned.as_str() != device.signing_public_key.as_str()
        {
            // Either the machine was reinstalled (regroup
            // regenerated its signing keys) or this is spoofing.
            let pair = (
                device.ssh_public_key.clone(),
                device.signing_public_key.clone(),
            );
            if !self.rejected_signing_keys.contains(&pair) && self.rotation_pending.is_none() {
                if ctx.dry_run {
                    eprintln!(
                        "\n[DRY RUN] {} announces with a NEW signing key — would ask to trust it",
                        device.name
                    );
                } else {
                    eprintln!(
                        "\n[!] {} announces with a NEW signing key (expected: {}…, received: {}…)",
                        device.name,
                        &pinned.as_str()[..8.min(pinned.as_str().len())],
                        &device.signing_public_key.as_str()
                            [..8.min(device.signing_public_key.as_str().len())]
                    );
                    println!(
                        "    This is expected after a reinstall (regroup regenerates its signing keys)."
                    );
                    println!("    If you do not expect a reinstall, this may be spoofing.");
                    print!("    Trust the new signing key for {}? [y/N]: ", device.name);
                    io::stdout().flush().ok();
                    self.reset_prompt_time();
                    self.rotation_pending = Some(device);
                }
            }
            return true;
        }

        // Known device?
        if self.known_keys.contains(&device.ssh_public_key) {
            // Check if IP changed before notifying
            if let Some(last_ip) = self.known_ip.get(&device.ssh_public_key)
                && last_ip == &device.ip
            {
                return true; // IP unchanged, skip silently
            }
            // Known device with new IP: verify the host key at the new IP
            // before trusting it, then update SSH config and known_hosts.
            // A mismatch means the announced IP does not serve this
            // machine's key (spoofing/MITM): keep the last verified IP.
            if let Some(machine) = self.key_to_machine.get(&device.ssh_public_key)
                && !ctx.dry_run
            {
                let verified = match public_key::compute_fingerprint_from_public_key(
                    machine.ssh_public_key.as_str(),
                ) {
                    Some(expected) => match public_key::verify_and_add_to_known_hosts(
                        &device.ip,
                        device.port,
                        &expected,
                    ) {
                        public_key::HostKeyVerification::Success => true,
                        public_key::HostKeyVerification::HostUnreachable => {
                            println!(
                                "\n[~] Known device: {} at {} — unreachable, keeping last verified IP",
                                machine.name, device.ip
                            );
                            false
                        }
                        public_key::HostKeyVerification::WriteError => {
                            // Verification passed; only the known_hosts write failed
                            eprintln!(
                                "WARNING: could not write {} to ~/.ssh/known_hosts",
                                machine.name
                            );
                            true
                        }
                        public_key::HostKeyVerification::FingerprintMismatch {
                            expected,
                            actual,
                        } => {
                            eprintln!(
                                "\n[!] SECURITY: {} announces IP {} but serves a DIFFERENT host key — possible spoofing/MITM. SSH config NOT updated.",
                                machine.name, device.ip
                            );
                            eprintln!("    expected fingerprint: {}", expected);
                            eprintln!("    actual fingerprint:   {}", actual);
                            false
                        }
                    },
                    None => {
                        eprintln!(
                            "WARNING: cannot compute fingerprint of {}'s host key — keeping last verified IP",
                            machine.name
                        );
                        false
                    }
                };
                if verified {
                    let _ = update_block(
                        ctx.ssh_config,
                        &machine.name,
                        &device.ip,
                        &machine.user,
                        machine.port,
                        machine.port_explicit,
                    );
                    println!(
                        "\n[~] Known device: {} at {} (IP updated, host key verified)",
                        machine.name, device.ip
                    );
                }
                // Remember this IP either way, so we do not
                // re-attempt (and re-warn about) the same IP on
                // every beacon.
                self.known_ip
                    .insert(device.ssh_public_key.clone(), device.ip.clone());
            }
            return true; // Don't add to pending queue
        }

        // New device: check for duplicates in pending queue and rejected list
        if !self.pending_keys.contains(&device.ssh_public_key)
            && !self.rejected_keys.contains(&device.ssh_public_key)
        {
            self.pending_keys.insert(device.ssh_public_key.clone());
            self.pending_devices.push_back(device);
            if !ctx.dry_run {
                let added_device = self.pending_devices.back().unwrap();
                println!(
                    "\n[!] New device: {} ({}) - key: {}",
                    added_device.name,
                    added_device.ip,
                    key_preview(&added_device.ssh_public_key)
                );
                println!("    Type 'y' to add, 'N' to skip, 'r' to refresh git, 'q' to quit");
            }
        }
        false
    }

    /// Handle one line of user input (when available). Returns true when the
    /// user asked to quit.
    fn handle_input(&mut self, ctx: &Context, user_command: Result<String, TryRecvError>) -> bool {
        if self.rename_state.is_some() {
            self.handle_rename_input(ctx, user_command);
            false
        } else if self.rotation_pending.is_some() {
            self.handle_rotation_input(ctx, user_command);
            false
        } else if !self.pending_devices.is_empty() {
            if ctx.dry_run {
                self.dry_run_next_device();
                false
            } else {
                self.handle_pending_input(ctx, user_command)
            }
        } else {
            self.handle_idle_input(ctx, user_command)
        }
    }

    /// If we are waiting for the user to pick a new name for a device, the
    /// next input line is that name (empty = suggested name).
    fn handle_rename_input(&mut self, ctx: &Context, user_command: Result<String, TryRecvError>) {
        if self.prompt_due(user_command.is_err()) {
            let (device, suggested) = self.rename_state.as_ref().unwrap();
            print!("\nNew name for {} (empty = {}): ", device.name, suggested);
            io::stdout().flush().ok();
            self.reset_prompt_time();
        }
        let Some(input) = user_command.ok() else {
            return;
        };
        self.reset_prompt_time();
        let input = input.trim();
        if input == "q" {
            println!("  Skipped {}.", self.rename_state.as_ref().unwrap().0.name);
            self.rename_state = None;
            return;
        }
        let (device, suggested) = self.rename_state.take().unwrap();
        let name = if input.is_empty() {
            suggested
        } else {
            input.to_string()
        };
        if device_name_taken(&name, ctx.home, ctx.ssh_config) {
            println!(
                "  ! '{}' is also already used by another machine or an existing ~/.ssh/config entry.",
                name
            );
            let new_suggested = unique_device_name(&name, ctx.home, ctx.ssh_config);
            println!("    Pick another name (empty = {}):", new_suggested);
            self.rename_state = Some((device, new_suggested));
        } else if let Some(machine) = add_device_to_config(
            ctx.home,
            ctx.ssh_config,
            &device,
            &name,
            &mut self.new_count,
        ) {
            // Remember the machine under its final name so future
            // beacons update the right ~/.ssh/config block.
            self.known_keys.insert(device.ssh_public_key.clone());
            self.key_to_machine
                .insert(device.ssh_public_key.clone(), machine);
            self.known_ip
                .insert(device.ssh_public_key.clone(), device.ip.clone());
        }
    }

    /// Waiting for the user to decide about a new signing key.
    fn handle_rotation_input(&mut self, ctx: &Context, user_command: Result<String, TryRecvError>) {
        if self.prompt_due(user_command.is_err()) {
            let device = self.rotation_pending.as_ref().unwrap();
            print!("\nTrust the new signing key for {}? [y/N]: ", device.name);
            io::stdout().flush().ok();
            self.reset_prompt_time();
        }
        let Some(input) = user_command.ok() else {
            return;
        };
        self.reset_prompt_time();
        let device = self.rotation_pending.take().unwrap();
        let input = input.trim().to_lowercase();
        if input == "y" || input == "yes" {
            self.pinned_signing_keys.insert(
                device.ssh_public_key.clone(),
                device.signing_public_key.clone(),
            );
            trust_new_signing_key(ctx.home, &device);
            println!(
                "  + New signing key for {} trusted — its next beacon will be processed normally.",
                device.name
            );
        } else {
            self.rejected_signing_keys.insert((
                device.ssh_public_key.clone(),
                device.signing_public_key.clone(),
            ));
            println!(
                "  New signing key for {} rejected — its beacons are ignored.",
                device.name
            );
        }
    }

    /// Prompt for the first pending device until answered. Returns true when
    /// the user asked to quit.
    fn handle_pending_input(
        &mut self,
        ctx: &Context,
        user_command: Result<String, TryRecvError>,
    ) -> bool {
        // Show prompt for the current device (first in queue) - FIFO
        // Re-display prompt periodically until answered
        if self.prompt_due(user_command.is_err()) {
            let device = &self.pending_devices[0];
            print!("\nAdd {} to config? [y/N/q/r]: ", device.name);
            io::stdout().flush().ok();
            self.reset_prompt_time();
        }

        let Some(input) = user_command.ok() else {
            return false;
        };
        self.reset_prompt_time();
        let device = self.pending_devices.pop_front().unwrap();
        let device_name = device.name.clone();
        let device_ssh_public_key = device.ssh_public_key.clone();
        self.pending_keys.remove(&device_ssh_public_key);
        let input = input.trim().to_lowercase();
        match input.as_str() {
            "y" | "yes" => {
                // If the name is already used by another machine,
                // ask the user to pick a new one.
                if device_name_taken(&device_name, ctx.home, ctx.ssh_config) {
                    println!(
                        "\n  ! Name '{}' is already used by another machine or an existing ~/.ssh/config entry.",
                        device_name
                    );
                    let suggested = unique_device_name(&device_name, ctx.home, ctx.ssh_config);
                    print!("  New name for this device (empty = {}): ", suggested);
                    io::stdout().flush().ok();
                    self.reset_prompt_time();
                    self.rename_state = Some((device, suggested));
                } else if let Some(machine) = add_device_to_config(
                    ctx.home,
                    ctx.ssh_config,
                    &device,
                    &device_name,
                    &mut self.new_count,
                ) {
                    // Pin the signing key the device used when accepted
                    self.pinned_signing_keys.insert(
                        device_ssh_public_key.clone(),
                        machine.signing_public_key.clone(),
                    );
                    self.known_keys.insert(device_ssh_public_key.clone());
                    self.key_to_machine
                        .insert(device_ssh_public_key.clone(), machine);
                    self.known_ip
                        .insert(device_ssh_public_key.clone(), device.ip.clone());
                }
            }
            "r" => {
                self.pending_devices.push_front(device);
                self.pending_keys.insert(device_ssh_public_key);
                refresh_git(ctx.home);
                // A pull may have brought new device configs: reload
                self.reload();
            }
            "q" => {
                println!();
                return true;
            }
            _ => {
                // Device rejected, remember to not ask again
                self.rejected_keys.insert(device_ssh_public_key);
                println!("  Skipped {}.", device_name);
            }
        }
        false
    }

    /// Dry run: show devices as they are detected, without asking anything.
    fn dry_run_next_device(&mut self) {
        let device = self.pending_devices.pop_front().unwrap();
        self.pending_keys.remove(&device.ssh_public_key);
        println!(
            "\n[DRY RUN] New device: {} ({}) - key: {}",
            device.name,
            device.ip,
            key_preview(&device.ssh_public_key)
        );
        self.new_count += 1;
    }

    /// No pending devices: process global commands, or show the idle prompt.
    /// Returns true when the user asked to quit.
    fn handle_idle_input(
        &mut self,
        ctx: &Context,
        user_command: Result<String, TryRecvError>,
    ) -> bool {
        let Some(input) = user_command.ok() else {
            if !self.waiting_prompt_shown {
                print!("Waiting for devices or command [q=quit, r=refresh git]: ");
                io::stdout().flush().ok();
                self.waiting_prompt_shown = true;
            }
            return false;
        };
        let input = input.trim().to_lowercase();
        match input.as_str() {
            "q" | "quit" => {
                println!();
                println!("Stopping...");
                return true;
            }
            "r" | "refresh" => {
                println!();
                // Dry run must not modify anything: skip git refresh
                if ctx.dry_run {
                    println!("  (dry run - git refresh skipped)");
                } else {
                    refresh_git(ctx.home);
                    // A pull may have brought new device configs: reload
                    self.reload();
                }
            }
            _ => {}
        }
        self.waiting_prompt_shown = false;
        false
    }
}

/// Short preview of an SSH public key for display, e.g. "ssh-ed25519 AA...".
fn key_preview(key: &SshPublicKey) -> String {
    let s = key.as_str();
    if s.len() > 16 {
        format!("{}...", &s[..13])
    } else {
        s.to_string()
    }
}
