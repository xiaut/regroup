//! Interactive mode: regroup's main loop.
//!
//! Listens for verified beacons (forwarded by the listener thread through a
//! channel) and for user input (read by the ui module), and manages the
//! pending-device queue, the signing-key rotation prompt and the rename
//! prompt. All decisions that affect what is written to disk live here.
//!
//! All output goes through `ui::log`; the ui renders it (activity pane of the
//! full-screen interface, stdout in plain mode) and keeps the current
//! question visible as the bottom block at all times.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crate::cli::{
    add_device_to_config, device_name_taken, refresh_git, trust_new_signing_key,
    unique_device_name,
};
use crate::config::{Machine, load_config};
use crate::discovery::beacon::BeaconDevice;
use crate::ssh::config::update_block;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};
use crate::ssh::public_key;
use crate::ui::{self, Action, Device, Question};

/// How long each loop iteration waits for user input before checking the
/// beacon channel again.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

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
/// signature-verified and age-checked by the listener). Returns the number
/// of devices accepted this run.
pub fn run(ctx: &Context, udp_port: u16, beacons: &Receiver<BeaconDevice>) -> usize {
    let mut ui = ui::Ui::start();
    ui::log(&format!("Listening for devices on UDP port {udp_port}..."));

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

        if let Some(action) = ui.poll_action(POLL_INTERVAL)
            && state.handle_action(ctx, action)
        {
            break;
        }

        // The display always reflects the state after processing.
        ui.set_question(state.current_question(ctx));
        ui.set_devices(state.devices_view());
        ui.draw();
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
    /// the ui then asks for the new name. Only reachable in interactive
    /// mode: dry run never adds devices.
    rename_state: Option<(BeaconDevice, String)>,
    /// Devices accepted this run.
    new_count: usize,
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

    /// The question to display, derived from the current state. Dry run
    /// never asks anything: pending devices are only logged.
    fn current_question(&self, ctx: &Context) -> Question {
        if let Some((device, suggested)) = &self.rename_state {
            Question::Rename {
                name: device.name.clone(),
                suggested: suggested.clone(),
            }
        } else if let Some(device) = &self.rotation_pending {
            Question::TrustKey {
                name: device.name.clone(),
            }
        } else if !ctx.dry_run
            && let Some(device) = self.pending_devices.front()
        {
            Question::AddDevice {
                name: device.name.clone(),
                ip: device.ip.clone(),
            }
        } else {
            Question::Idle
        }
    }

    /// The machines to show in the devices panel: known machines with their
    /// last known IP, plus pending devices marked as pending.
    fn devices_view(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self
            .key_to_machine
            .values()
            .map(|m| Device {
                name: m.name.clone(),
                ip: self.known_ip.get(&m.ssh_public_key).cloned(),
                port: m.port,
                pending: false,
            })
            .collect();
        for device in &self.pending_devices {
            if !devices.iter().any(|d| d.name == device.name) {
                devices.push(Device {
                    name: device.name.clone(),
                    ip: Some(device.ip.clone()),
                    port: device.port,
                    pending: true,
                });
            }
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
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
                    ui::log(&format!(
                        "[DRY RUN] {} announces with a NEW signing key — would ask to trust it",
                        device.name
                    ));
                } else {
                    ui::log(&format!(
                        "[!] {} announces with a NEW signing key (expected: {}…, received: {}…)\n    This is expected after a reinstall (regroup regenerates its signing keys).\n    If you do not expect a reinstall, this may be spoofing.",
                        device.name,
                        &pinned.as_str()[..8.min(pinned.as_str().len())],
                        &device.signing_public_key.as_str()
                            [..8.min(device.signing_public_key.as_str().len())]
                    ));
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
                            ui::log(&format!(
                                "[~] Known device: {} at {} — unreachable, keeping last verified IP",
                                machine.name, device.ip
                            ));
                            false
                        }
                        public_key::HostKeyVerification::WriteError => {
                            // Verification passed; only the known_hosts write failed
                            ui::log(&format!(
                                "WARNING: could not write {} to ~/.ssh/known_hosts",
                                machine.name
                            ));
                            true
                        }
                        public_key::HostKeyVerification::FingerprintMismatch {
                            expected,
                            actual,
                        } => {
                            ui::log(&format!(
                                "[!] SECURITY: {} announces IP {} but serves a DIFFERENT host key — possible spoofing/MITM. SSH config NOT updated.\n    expected fingerprint: {}\n    actual fingerprint:   {}",
                                machine.name, device.ip, expected, actual
                            ));
                            false
                        }
                    },
                    None => {
                        ui::log(&format!(
                            "WARNING: cannot compute fingerprint of {}'s host key — keeping last verified IP",
                            machine.name
                        ));
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
                    ui::log(&format!(
                        "[~] Known device: {} at {} (IP updated, host key verified)",
                        machine.name, device.ip
                    ));
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
        if self.pending_keys.contains(&device.ssh_public_key)
            || self.rejected_keys.contains(&device.ssh_public_key)
        {
            return false;
        }
        self.pending_keys.insert(device.ssh_public_key.clone());
        if ctx.dry_run {
            self.new_count += 1;
            ui::log(&format!(
                "[DRY RUN] New device: {} ({}) - key: {}",
                device.name,
                device.ip,
                key_preview(&device.ssh_public_key)
            ));
            return false;
        }
        self.pending_devices.push_back(device);
        let added_device = self.pending_devices.back().unwrap();
        ui::log(&format!(
            "[!] New device: {} ({}) - key: {}",
            added_device.name,
            added_device.ip,
            key_preview(&added_device.ssh_public_key)
        ));
        false
    }

    /// Apply a user decision. Returns true when the user asked to quit.
    fn handle_action(&mut self, ctx: &Context, action: Action) -> bool {
        match action {
            Action::Quit => return true,
            Action::Refresh => {
                // Dry run must not modify anything: skip git refresh
                if ctx.dry_run {
                    ui::log("  (dry run - git refresh skipped)");
                } else {
                    refresh_git(ctx.home);
                    // A pull may have brought new device configs: reload
                    self.reload();
                }
            }
            Action::AddCurrent => self.accept_current(ctx),
            Action::SkipCurrent => {
                if let Some(device) = self.pending_devices.pop_front() {
                    self.pending_keys.remove(&device.ssh_public_key);
                    // Device rejected, remember to not ask again
                    self.rejected_keys.insert(device.ssh_public_key);
                    ui::log(&format!("  Skipped {}.", device.name));
                }
            }
            Action::TrustKey => {
                if let Some(device) = self.rotation_pending.take() {
                    self.pinned_signing_keys.insert(
                        device.ssh_public_key.clone(),
                        device.signing_public_key.clone(),
                    );
                    trust_new_signing_key(ctx.home, &device);
                    ui::log(&format!(
                        "  + New signing key for {} trusted — its next beacon will be processed normally.",
                        device.name
                    ));
                }
            }
            Action::RejectKey => {
                if let Some(device) = self.rotation_pending.take() {
                    self.rejected_signing_keys.insert((
                        device.ssh_public_key.clone(),
                        device.signing_public_key.clone(),
                    ));
                    ui::log(&format!(
                        "  New signing key for {} rejected — its beacons are ignored.",
                        device.name
                    ));
                }
            }
            Action::Rename(name) => self.rename(ctx, name),
            Action::SkipRename => {
                if let Some((device, _)) = self.rename_state.take() {
                    ui::log(&format!("  Skipped {}.", device.name));
                }
            }
        }
        false
    }

    /// Accept the pending device at the front of the queue. A name collision
    /// redirects to the rename question instead.
    fn accept_current(&mut self, ctx: &Context) {
        let Some(device) = self.pending_devices.pop_front() else {
            return;
        };
        self.pending_keys.remove(&device.ssh_public_key);
        // If the name is already used by another machine,
        // ask the user to pick a new one.
        if device_name_taken(&device.name, ctx.home, ctx.ssh_config) {
            let suggested = unique_device_name(&device.name, ctx.home, ctx.ssh_config);
            ui::log(&format!(
                "  ! Name '{}' is already used by another machine or an existing ~/.ssh/config entry.",
                device.name
            ));
            self.rename_state = Some((device, suggested));
            return;
        }
        let name = device.name.clone();
        self.add_device(ctx, device, name);
    }

    /// The user picked a new name for the pending rename. A name that is
    /// taken as well re-asks with a fresh suggestion.
    fn rename(&mut self, ctx: &Context, name: String) {
        let Some((device, _)) = self.rename_state.take() else {
            return;
        };
        if device_name_taken(&name, ctx.home, ctx.ssh_config) {
            let suggested = unique_device_name(&name, ctx.home, ctx.ssh_config);
            ui::log(&format!(
                "  ! '{name}' is also already used by another machine or an existing ~/.ssh/config entry."
            ));
            self.rename_state = Some((device, suggested));
            return;
        }
        self.add_device(ctx, device, name);
    }

    /// Add `device` under `name`: write to disk and update the in-memory
    /// maps so future beacons for it are treated as a known device.
    fn add_device(&mut self, ctx: &Context, device: BeaconDevice, name: String) {
        if let Some(machine) = add_device_to_config(
            ctx.home,
            ctx.ssh_config,
            &device,
            &name,
            &mut self.new_count,
        ) {
            // Pin the signing key the device used when accepted
            self.pinned_signing_keys.insert(
                device.ssh_public_key.clone(),
                machine.signing_public_key.clone(),
            );
            self.known_keys.insert(device.ssh_public_key.clone());
            self.key_to_machine
                .insert(device.ssh_public_key.clone(), machine);
            self.known_ip
                .insert(device.ssh_public_key.clone(), device.ip.clone());
        }
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
