//! Interactive mode: regroup's main loop.
//!
//! Listens for verified beacons (forwarded by the listener thread through a
//! channel) and for user input (read by the ui module), and manages the
//! pending-device queue, the signing-key rotation prompt and the rename
//! prompt. All decisions that affect what is written to disk live here.
//!
//! General messages go through `ui::log` (activity pane of the full-screen
//! interface, stdout in plain mode); messages about one machine go through
//! `ui::log_device`, which the full-screen ui shows as an animated status on
//! the machine's row. The current question stays visible as the bottom block
//! at all times.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crate::cli::{
    add_device_to_config, device_name_taken, remove_device, remove_local_traces, setup_cd,
    shell_rc_file, sync_git, trust_new_signing_key, unique_device_name, usage,
    wipe as wipe_config,
};
use crate::config::{CONFIG_DIR, GIT_DIR, Machine, load_config};
use crate::discovery::beacon::BeaconDevice;
use crate::ssh::config::update_block;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};
use crate::ssh::public_key;
use crate::ui::{self, Action, Device, Level, Question};

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
    /// Initial verbose state (-v): extra information in the activity log;
    /// the user can toggle it at runtime with 'v'.
    pub verbose: bool,
    /// This machine's hostname, for the verbose identity line.
    pub local_name: &'a str,
    /// This machine's IP, for the verbose identity line.
    pub local_ip: Option<&'a str>,
}

/// Run the interactive loop until the user quits or the listener thread dies.
///
/// `beacons` carries devices announced by other machines (already
/// signature-verified and age-checked by the listener). Returns the number
/// of devices accepted this run.
pub fn run(ctx: &Context, udp_port: u16, beacons: &Receiver<BeaconDevice>) -> usize {
    let mut ui = ui::Ui::start();
    let mut state = InteractiveState::new(ctx);
    ui::log(&format!("Listening for devices on UDP port {udp_port}..."));
    // Verbose identity line: through ui::log so the full-screen ui shows it
    // in the activity pane instead of losing it to the alternate screen.
    if state.verbose {
        let who = format!(
            "{} / {}",
            ctx.local_name,
            ctx.local_ip.unwrap_or("unknown IP")
        );
        if ctx.local_public_key.is_empty() {
            ui::log(&format!(
                "  . this machine: {who} (NO SSH SERVER - not announcing)"
            ));
        } else {
            let key = ctx.local_public_key.as_str();
            ui::log(&format!(
                "  . this machine: {who} (SSH key: {})",
                &key[..16.min(key.len())]
            ));
        }
    }

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
        let plain = !matches!(ui, ui::Ui::Tty(_));
        ui.set_question(state.current_question(ctx, plain));
        let detected = state.detected_view();
        let added = state.added_view();
        let selected = state
            .pending_selected
            .as_ref()
            .and_then(|key| detected.iter().position(|d| d.key == key.as_str()));
        let delete_selected = state
            .delete_selection
            .as_ref()
            .and_then(|name| added.iter().position(|d| &d.name == name));
        ui.set_devices(detected, added);
        ui.set_selected(selected);
        ui.set_delete_selected(delete_selected);
        ui.set_verbose(state.verbose);
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
    /// Machine selected in the Detected panel (full-screen ui), by public
    /// key. None when the queue is empty.
    pending_selected: Option<SshPublicKey>,
    /// Last known IP for known devices, to avoid redundant notifications.
    known_ip: HashMap<SshPublicKey, String>,
    /// Devices the user rejected, to avoid re-asking.
    rejected_keys: HashSet<SshPublicKey>,
    /// Device waiting for the user to pick a new name, plus the suggested
    /// name. Set when the user accepts a device whose name is already taken;
    /// the ui then asks for the new name. Only reachable in interactive
    /// mode: dry run never adds devices.
    rename_state: Option<(BeaconDevice, String)>,
    /// Device selected in delete mode (key 'd'), by name. None = not in
    /// delete mode.
    delete_selection: Option<String>,
    /// Wipe confirmation is pending (key 'w'): the ui asks for a final
    /// confirm before the whole configuration is deleted.
    wipe_pending: bool,
    /// Setup-cd confirmation is pending (key 'c'): the ui asks for a final
    /// confirm before the 'regroup-cd' shell function is installed.
    setup_cd_pending: bool,
    /// Install key mode: select target machine and user
    install_key_mode: bool,
    /// Selected machine for key installation
    install_key_target: Option<String>,
    /// Selected user for key installation
    install_key_user: Option<String>,
    /// Remove key mode: select connection to remove
    remove_key_mode: bool,
    /// Selected connection for key removal
    remove_key_connection: Option<String>,
    /// Verbose mode: extra information in the activity log. Initialized
    /// from the -v flag, toggled at runtime with 'v'.
    verbose: bool,
    /// Devices accepted this run.
    new_count: usize,
}

impl InteractiveState {
    fn new(ctx: &Context) -> Self {
        let mut state = Self {
            known_keys: HashSet::new(),
            key_to_machine: HashMap::new(),
            pinned_signing_keys: HashMap::new(),
            rejected_signing_keys: HashSet::new(),
            rotation_pending: None,
            pending_devices: VecDeque::new(),
            pending_keys: HashSet::new(),
            pending_selected: None,
            known_ip: HashMap::new(),
            rejected_keys: HashSet::new(),
            rename_state: None,
            delete_selection: None,
            wipe_pending: false,
            setup_cd_pending: false,
            install_key_mode: false,
            install_key_target: None,
            install_key_user: None,
            remove_key_mode: false,
            remove_key_connection: None,
            verbose: ctx.verbose,
            new_count: 0,
        };
        state.reload(ctx);
        state
    }

    /// Rebuild the lookup maps from the device files on disk (e.g. after a
    /// git sync pulled new device files).
    ///
    /// Machines that disappeared from the repository were removed on
    /// another machine: their local traces (~/.ssh/config block, git remote,
    /// last known_hosts IP) are cleaned up so the deletion propagates.
    fn reload(&mut self, ctx: &Context) {
        let old = std::mem::take(&mut self.key_to_machine);
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
        for (key, machine) in old {
            if !self.key_to_machine.contains_key(&key) {
                let last_ip = self.known_ip.get(&key).cloned();
                remove_local_traces(ctx.home, ctx.ssh_config, &machine, last_ip.as_deref());
                ui::log(&format!(
                    "  - {} was removed from the configuration (synced) — local traces cleaned",
                    machine.name
                ));
                self.known_ip.remove(&key);
            }
        }
    }

    /// The question to display, derived from the current state. Dry run
    /// never asks anything: pending devices are only logged. In plain mode
    /// the pending queue is asked one device at a time; the full-screen ui
    /// adds machines from the Detected panel instead.
    fn current_question(&self, ctx: &Context, plain: bool) -> Question {
        if self.wipe_pending {
            Question::Wipe {
                count: self.key_to_machine.len(),
            }
        } else if self.setup_cd_pending {
            Question::SetupCd {
                file: shell_rc_file(ctx.home)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            }
        } else if let Some(name) = &self.delete_selection {
            Question::Delete {
                name: name.clone(),
            }
        } else if let Some((device, suggested)) = &self.rename_state {
            Question::Rename {
                name: device.name.clone(),
                suggested: suggested.clone(),
            }
        } else if let Some(device) = &self.rotation_pending {
            Question::TrustKey {
                name: device.name.clone(),
            }
        } else if self.install_key_mode {
            // For now, use a simple question - full interactive selection comes later
            Question::InstallKeyConfirm {
                from_user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
                from_machine: ctx.local_name.to_string(),
                to_user: self.install_key_user.clone().unwrap_or_else(|| "unknown".to_string()),
                to_machine: self.install_key_target.clone().unwrap_or_else(|| "unknown".to_string()),
            }
        } else if self.remove_key_mode {
            // For now, use a simple question - full interactive selection comes later
            Question::RemoveKeyConfirm {
                connection_id: self.remove_key_connection.clone().unwrap_or_else(|| "unknown".to_string()),
                from_user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
                from_machine: ctx.local_name.to_string(),
                to_user: "unknown".to_string(),
                to_machine: "unknown".to_string(),
            }
        } else if plain
            && !ctx.dry_run
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

    /// The machines to show in the Detected panel: announced machines
    /// waiting for a decision, sorted by name.
    fn detected_view(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self
            .pending_devices
            .iter()
            .map(|d| Device {
                key: d.ssh_public_key.as_str().to_string(),
                name: d.name.clone(),
                ip: Some(d.ip.clone()),
                port: d.port,
            })
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }

    /// The machines to show in the Added panel: known machines with their
    /// last known IP, sorted by name.
    fn added_view(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self
            .key_to_machine
            .values()
            .map(|m| Device {
                key: m.ssh_public_key.as_str().to_string(),
                name: m.name.clone(),
                ip: self.known_ip.get(&m.ssh_public_key).cloned(),
                port: m.port,
            })
            .collect();
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

        // Every announcement from another machine pulses its row, so the
        // user sees the app receiving — including silent beacons from known
        // machines with an unchanged IP.
        ui::pulse_device(&device.ssh_public_key);

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
                    ui::log_device(
                        &device.ssh_public_key,
                        &device.name,
                        Level::Warning,
                        "NEW signing key — reinstall or spoofing?",
                        Some(&format!(
                            "[!] {} announces with a NEW signing key (expected: {}…, received: {}…)\n    This is expected after a reinstall (regroup regenerates its signing keys).\n    If you do not expect a reinstall, this may be spoofing.",
                            device.name,
                            &pinned.as_str()[..8.min(pinned.as_str().len())],
                            &device.signing_public_key.as_str()
                                [..8.min(device.signing_public_key.as_str().len())]
                        )),
                    );
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
                            ui::log_device(
                                &machine.ssh_public_key,
                                &machine.name,
                                Level::Warning,
                                "unreachable — keeping last verified IP",
                                Some(&format!(
                                    "[~] Known device: {} at {} — unreachable, keeping last verified IP",
                                    machine.name, device.ip
                                )),
                            );
                            false
                        }
                        public_key::HostKeyVerification::WriteError => {
                            // Verification passed; only the known_hosts write failed
                            ui::log_device(
                                &machine.ssh_public_key,
                                &machine.name,
                                Level::Warning,
                                "verified, but known_hosts write failed",
                                Some(&format!(
                                    "WARNING: could not write {} to ~/.ssh/known_hosts",
                                    machine.name
                                )),
                            );
                            true
                        }
                        public_key::HostKeyVerification::FingerprintMismatch {
                            expected,
                            actual,
                        } => {
                            ui::log_device(
                                &machine.ssh_public_key,
                                &machine.name,
                                Level::Security,
                                "host key MISMATCH — possible spoofing/MITM",
                                Some(&format!(
                                    "[!] SECURITY: {} announces IP {} but serves a DIFFERENT host key — possible spoofing/MITM. SSH config NOT updated.\n    expected fingerprint: {}\n    actual fingerprint:   {}",
                                    machine.name, device.ip, expected, actual
                                )),
                            );
                            false
                        }
                    },
                    None => {
                        ui::log_device(
                            &machine.ssh_public_key,
                            &machine.name,
                            Level::Warning,
                            "cannot compute fingerprint — keeping last verified IP",
                            Some(&format!(
                                "WARNING: cannot compute fingerprint of {}'s host key — keeping last verified IP",
                                machine.name
                            )),
                        );
                        false
                    }
                };
                if verified {
                    let old_ip = self.known_ip.get(&device.ssh_public_key).cloned();
                    let _ = update_block(
                        ctx.ssh_config,
                        &machine.name,
                        &device.ip,
                        &machine.user,
                        machine.port,
                        machine.port_explicit,
                    );
                    // Clean the known_hosts entries of the old IP so stale
                    // addresses do not linger — unless another known device
                    // has taken it over in the meantime.
                    if let Some(old_ip) = old_ip
                        && old_ip != device.ip
                        && !self.known_ip.iter().any(|(k, ip)| {
                            k != &device.ssh_public_key && ip == &old_ip
                        })
                        && public_key::remove_known_host(&old_ip) > 0
                    {
                        ui::log_device(
                            &machine.ssh_public_key,
                            &machine.name,
                            Level::Info,
                            "cleaned old known_hosts entries",
                            Some(&format!(
                                "  ~ Cleaned old known_hosts entries for {old_ip}"
                            )),
                        );
                    }
                    ui::log_device(
                        &machine.ssh_public_key,
                        &machine.name,
                        Level::Success,
                        "IP updated — host key verified",
                        Some(&format!(
                            "[~] Known device: {} at {} (IP updated, host key verified)",
                            machine.name, device.ip
                        )),
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
        // Select the new machine so it can be added right away.
        if self.pending_selected.is_none() {
            self.pending_selected = Some(self.pending_devices.back().unwrap().ssh_public_key.clone());
        }
        let added_device = self.pending_devices.back().unwrap();
        ui::log_device(
            &added_device.ssh_public_key,
            &added_device.name,
            Level::Info,
            "new device detected",
            Some(&format!(
                "[!] New device: {} ({}) - key: {}",
                added_device.name,
                added_device.ip,
                key_preview(&added_device.ssh_public_key)
            )),
        );
        false
    }

    /// Apply a user decision. Returns true when the user asked to quit.
    fn handle_action(&mut self, ctx: &Context, action: Action) -> bool {
        match action {
            Action::Quit => return true,
            Action::Sync => {
                // Dry run must not modify anything: skip git sync
                if ctx.dry_run {
                    ui::log("  (dry run - git sync skipped)");
                } else {
                    sync_git(ctx.home, self.verbose);
                    // A pull may have brought new device configs: reload
                    self.reload(ctx);
                }
            }
            Action::Add => self.accept_selected(ctx),
            Action::Ignore => self.ignore_selected(),
            Action::AddCurrent => self.accept_front(ctx),
            Action::SkipCurrent => {
                if let Some(device) = self.pending_devices.pop_front() {
                    self.pending_keys.remove(&device.ssh_public_key);
                    // Device rejected, remember to not ask again
                    self.rejected_keys.insert(device.ssh_public_key);
                    ui::log(&format!("  Skipped {}.", device.name));
                }
                self.after_selection_change();
            }
            Action::TrustKey => {
                if let Some(device) = self.rotation_pending.take() {
                    self.pinned_signing_keys.insert(
                        device.ssh_public_key.clone(),
                        device.signing_public_key.clone(),
                    );
                    trust_new_signing_key(ctx.home, &device);
                    ui::log_device(
                        &device.ssh_public_key,
                        &device.name,
                        Level::Success,
                        "new signing key trusted",
                        Some(&format!(
                            "  + New signing key for {} trusted — its next beacon will be processed normally.",
                            device.name
                        )),
                    );
                }
            }
            Action::RejectKey => {
                if let Some(device) = self.rotation_pending.take() {
                    self.rejected_signing_keys.insert((
                        device.ssh_public_key.clone(),
                        device.signing_public_key.clone(),
                    ));
                    ui::log_device(
                        &device.ssh_public_key,
                        &device.name,
                        Level::Warning,
                        "new signing key rejected",
                        Some(&format!(
                            "  New signing key for {} rejected — its beacons are ignored.",
                            device.name
                        )),
                    );
                }
            }
            Action::Rename(name) => self.rename(ctx, name),
            Action::SkipRename => {
                if let Some((device, _)) = self.rename_state.take() {
                    ui::log(&format!("  Skipped {}.", device.name));
                }
            }
            Action::DeleteMode => {
                if ctx.dry_run {
                    ui::log("  (dry run - device removal skipped)");
                } else {
                    // Select the first deletable device, if any.
                    self.move_delete_selection(true);
                }
            }
            Action::SelectPrev => {
                if self.delete_selection.is_some() {
                    self.move_delete_selection(false);
                } else {
                    self.move_pending_selection(false);
                }
            }
            Action::SelectNext => {
                if self.delete_selection.is_some() {
                    self.move_delete_selection(true);
                } else {
                    self.move_pending_selection(true);
                }
            }
            Action::DeleteConfirm => self.delete_selected(ctx),
            Action::DeleteCancel => self.delete_selection = None,
            Action::WipeMode => {
                if ctx.dry_run {
                    ui::log("  (dry run - wipe skipped)");
                } else {
                    self.wipe_pending = true;
                }
            }
            Action::WipeCancel => self.wipe_pending = false,
            Action::WipeConfirm => {
                self.wipe_pending = false;
                let machines: Vec<Machine> = self.key_to_machine.values().cloned().collect();
                wipe_config(ctx.home, ctx.ssh_config, &machines);
                // Drop the wiped machines from memory, removing their
                // known_hosts entries (last known IP) on the way — wipe
                // itself leaves known_hosts alone (see its doc comment).
                for (key, _) in std::mem::take(&mut self.key_to_machine) {
                    if let Some(ip) = self.known_ip.remove(&key) {
                        public_key::remove_known_host(&ip);
                    }
                    self.known_keys.remove(&key);
                    self.pinned_signing_keys.remove(&key);
                }
                // Recreate the empty configuration skeleton (config dir,
                // git repository) so devices can be added again right away.
                load_config();
                ui::log("  - Configuration wiped — continuing with an empty configuration.");
            }
            Action::SetupCdMode => {
                if ctx.dry_run {
                    ui::log("  (dry run - setup-cd skipped)");
                } else {
                    self.setup_cd_pending = true;
                }
            }
            Action::SetupCdCancel => self.setup_cd_pending = false,
            Action::ToggleVerbose => {
                self.verbose = !self.verbose;
                ui::log(if self.verbose {
                    "  . Verbose mode on — identity and git sync details in the activity log"
                } else {
                    "  . Verbose mode off"
                });
            }
            Action::SetupCdConfirm => {
                self.setup_cd_pending = false;
                match setup_cd(ctx.home) {
                    Ok(rc) => {
                        let file = rc.file_name().unwrap_or_default().to_string_lossy();
                        ui::log(&format!(
                            "  + 'regroup-cd' installed in ~/{file} — run 'source ~/{file}' \
                             or open a new terminal, then use: regroup-cd"
                        ));
                    }
                    Err(e) => ui::log(&format!("  ! could not install 'regroup-cd': {e}")),
                }
            }
            // Plain mode only: the full-screen ui shows its help panel
            // instead, without going through an action.
            Action::Help => usage(),
            Action::InstallKeyMode => {
                if ctx.dry_run {
                    ui::log("  (dry run - key installation skipped)");
                } else {
                    self.execute_install_key(ctx);
                }
            }
            Action::InstallKeyConfirm => {
                // Not used in current simple implementation
            }
            Action::InstallKeyCancel => {
                // Not used in current simple implementation
            }
            Action::RemoveKeyMode => {
                if ctx.dry_run {
                    ui::log("  (dry run - key removal skipped)");
                } else {
                    self.execute_remove_key(ctx);
                }
            }
            Action::RemoveKeyConfirm => {
                // Not used in current simple implementation
            }
            Action::RemoveKeyCancel => {
                // Not used in current simple implementation
            }
            Action::ConnectionsMode => {
                // Connections management is notified only: the connections
                // browser (crate::tui::connections) is not wired into the
                // main loop yet (handing the terminal to a nested ratatui
                // loop and restoring the main drawing state needs care), and
                // connection creation/removal procedures are unimplemented.
                ui::log(
                    "Gestion des connexions : consultation via devices/<nom>/connections/ \
                     dans le dépôt git ; création et suppression non implémentées.",
                );
            }
        }
        false
    }

    /// Sorted names of the devices that can be deleted (known ones).
    fn deletable_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .key_to_machine
            .values()
            .map(|m| m.name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Move the delete-mode selection (Added panel) to the next (or
    /// previous when `forward` is false) known device. With no selection,
    /// selects the first one.
    fn move_delete_selection(&mut self, forward: bool) {
        let names = self.deletable_names();
        if names.is_empty() {
            self.delete_selection = None;
            return;
        }
        let pos = names
            .iter()
            .position(|n| Some(n) == self.delete_selection.as_ref());
        let new_pos = match pos {
            Some(p) if forward => (p + 1).min(names.len() - 1),
            Some(p) => p.saturating_sub(1),
            None => 0,
        };
        self.delete_selection = Some(names[new_pos].clone());
    }

    /// Move the selection of the Detected panel to the next (or previous
    /// when `forward` is false) pending machine, in name order. With no
    /// selection, selects the first one.
    fn move_pending_selection(&mut self, forward: bool) {
        let mut sorted: Vec<&BeaconDevice> = self.pending_devices.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        if sorted.is_empty() {
            self.pending_selected = None;
            return;
        }
        let pos = sorted
            .iter()
            .position(|d| Some(&d.ssh_public_key) == self.pending_selected.as_ref());
        let new_pos = match pos {
            Some(p) if forward => (p + 1).min(sorted.len() - 1),
            Some(p) => p.saturating_sub(1),
            None => 0,
        };
        self.pending_selected = Some(sorted[new_pos].ssh_public_key.clone());
    }

    /// Point the Detected-panel selection at a sensible machine after the
    /// selected one left the queue (added or ignored).
    fn after_selection_change(&mut self) {
        self.pending_selected = None;
        if let Some(front) = self.pending_devices.front() {
            self.pending_selected = Some(front.ssh_public_key.clone());
        }
    }

    /// Remove the selected device: delete its config file (committed, so
    /// the removal propagates on git sync), its ~/.ssh/config block, its
    /// known_hosts entries (last known IP) and its git remote, then drop
    /// it from the in-memory maps.
    fn delete_selected(&mut self, ctx: &Context) {
        let Some(name) = self.delete_selection.take() else {
            return;
        };
        let Some(machine) = self
            .key_to_machine
            .values()
            .find(|m| m.name == name)
            .cloned()
        else {
            return;
        };
        let key = machine.ssh_public_key.clone();
        let last_ip = self.known_ip.get(&key).cloned();
        if remove_device(ctx.home, ctx.ssh_config, &machine, last_ip.as_deref()) {
            ui::log(&format!(
                "  - Removed {} (device file, ~/.ssh/config block, git remote)",
                machine.name
            ));
        } else {
            ui::log(&format!(
                "  ! Could not fully remove {} — see warnings",
                machine.name
            ));
        }
        self.known_keys.remove(&key);
        self.key_to_machine.remove(&key);
        self.pinned_signing_keys.remove(&key);
        self.known_ip.remove(&key);
    }

    /// Add the machine selected in the Detected panel. A name collision
    /// redirects to the rename question instead.
    fn accept_selected(&mut self, ctx: &Context) {
        let Some(key) = self
            .pending_selected
            .clone()
            .or_else(|| self.pending_devices.front().map(|d| d.ssh_public_key.clone()))
        else {
            return;
        };
        let Some(pos) = self
            .pending_devices
            .iter()
            .position(|d| d.ssh_public_key == key)
        else {
            return;
        };
        let device = self.pending_devices.remove(pos).unwrap();
        self.pending_keys.remove(&device.ssh_public_key);
        self.after_selection_change();
        self.accept_device(ctx, device);
    }

    /// Add the front of the pending queue (plain mode). A name collision
    /// redirects to the rename question instead.
    fn accept_front(&mut self, ctx: &Context) {
        let Some(device) = self.pending_devices.pop_front() else {
            return;
        };
        self.pending_keys.remove(&device.ssh_public_key);
        self.after_selection_change();
        self.accept_device(ctx, device);
    }

    /// Accept `device`: a name collision redirects to the rename question.
    fn accept_device(&mut self, ctx: &Context, device: BeaconDevice) {
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

    /// Ignore the machine selected in the Detected panel: it is dropped
    /// from the queue and its beacons are ignored for the rest of the run.
    fn ignore_selected(&mut self) {
        let Some(key) = self
            .pending_selected
            .clone()
            .or_else(|| self.pending_devices.front().map(|d| d.ssh_public_key.clone()))
        else {
            return;
        };
        let Some(pos) = self
            .pending_devices
            .iter()
            .position(|d| d.ssh_public_key == key)
        else {
            return;
        };
        let device = self.pending_devices.remove(pos).unwrap();
        self.pending_keys.remove(&device.ssh_public_key);
        // Device rejected, remember to not ask again
        self.rejected_keys.insert(device.ssh_public_key);
        ui::log(&format!(
            "  Ignored {} — its beacons are ignored for this run.",
            device.name
        ));
        self.after_selection_change();
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
    
    /// Execute key installation on the first known machine
    fn execute_install_key(&mut self, ctx: &Context) {
        let config_dir = Path::new(ctx.home).join(CONFIG_DIR).join(GIT_DIR);
        let current_user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        
        // Get the first known machine (or selected one)
        let machines: Vec<Machine> = self.key_to_machine.values().cloned().collect();
        if machines.is_empty() {
            ui::log("No known machines to install a key to");
            return;
        }
        
        let target_machine = &machines[0].name;
        let target_user = if machines[0].user.is_empty() {
            current_user.clone()
        } else {
            machines[0].user.clone()
        };
        
        ui::log(&format!("Installing key to {}@{}", target_user, target_machine));
        
        let result = crate::tui::key_actions::install_key_action(
            ctx.home,
            &config_dir,
            ctx.local_name,
            &current_user,
            target_machine,
            &target_user,
        );
        
        match result {
            Ok(crate::tui::key_actions::KeyActionResult::Success(msg)) => {
                ui::log(&format!("Success: {}", msg));
            }
            Ok(crate::tui::key_actions::KeyActionResult::Error(msg)) => {
                ui::log(&format!("Error: {}", msg));
            }
            Ok(crate::tui::key_actions::KeyActionResult::Cancelled) => {
                ui::log("Installation cancelled");
            }
            Err(e) => {
                ui::log(&format!("Error: {}", e));
            }
        }
    }
    
    /// Execute key removal from the first known machine
    fn execute_remove_key(&mut self, ctx: &Context) {
        let config_dir = Path::new(ctx.home).join(CONFIG_DIR).join(GIT_DIR);
        let current_user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        
        // Find first outgoing connection
        let connections_dir = config_dir.join("devices").join(ctx.local_name).join("connections");
        if !connections_dir.exists() {
            ui::log("No outgoing connections found");
            return;
        }
        
        // Find first connection file
        if let Ok(entries) = std::fs::read_dir(&connections_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // Check if this is an outgoing connection (from current_user)
                    if filename.starts_with(&format!("{}->", current_user)) {
                        let connection_id = filename.trim_end_matches(".json").to_string();
                        let (from_user, to_user, to_machine) = crate::cli::key_management::parse_connection_id(&connection_id)
                            .unwrap_or_default();
                        
                        ui::log(&format!("Removing connection: {}", connection_id));
                        
                        let result = crate::tui::key_actions::remove_key_action(
                            ctx.home,
                            &config_dir,
                            ctx.local_name,
                            &current_user,
                            &from_user,
                            &to_machine,
                            &to_user,
                            &connection_id,
                        );
                        
                        match result {
                            Ok(crate::tui::key_actions::KeyActionResult::Success(msg)) => {
                                ui::log(&format!("Success: {}", msg));
                                return;
                            }
                            Ok(crate::tui::key_actions::KeyActionResult::Error(msg)) => {
                                ui::log(&format!("Error: {}", msg));
                                return;
                            }
                            Ok(crate::tui::key_actions::KeyActionResult::Cancelled) => {
                                ui::log("Removal cancelled");
                                return;
                            }
                            Err(e) => {
                                ui::log(&format!("Error: {}", e));
                                return;
                            }
                        }
                    }
                }
            }
        }
        
        ui::log("No outgoing connections found for this user");
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
