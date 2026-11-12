//! Interactive mode: regroup's main loop.
//!
//! Listens for verified beacons (forwarded by the listener thread through a
//! channel) and for user input (read by the ui module), and manages the
//! pending-device queue, the signing-key rotation prompt and the rename
//! prompt. All decisions that affect what is written to disk live here.
//!
//! General messages go through `ui::log` (activity pane of the full-screen
//! interface, stdout in plain mode); messages about one device go through
//! `ui::log_device`, which the full-screen ui shows as an animated status on
//! the device's row. The current question stays visible as the bottom block
//! at all times.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crate::cli::{NameConflict,
    add_device_to_config, device_name_taken, find_name_conflicts, remove_device, remove_local_traces, setup_cd,
    shell_rc_file, sync_git, trust_new_signing_key, unique_device_name, usage,
    wipe as wipe_config,
};
use crate::config::{CONFIG_DIR, GIT_DIR, Device, load_config};
use crate::config::users::load_device_users;
use crate::discovery::beacon::BeaconDevice;
use crate::ssh::config::update_block;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};
use crate::ssh::public_key;
use crate::ui::{self, Action, Level, Question};
use crate::ui::types::{ActivePane, NavDepth, NavFocus, Navigation, UiDevice, UiUser, UserStatus};

/// How long each loop iteration waits for user input before checking the
/// beacon channel again.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Format name conflicts into a human-readable message
fn format_name_conflicts(conflicts: &[NameConflict]) -> String {
    if conflicts.is_empty() {
        return "already in use".to_string();
    }
    
    let messages: Vec<String> = conflicts
        .iter()
        .map(|conflict| {
            match conflict {
                NameConflict::RegroupDevice { device_name } => {
                    format!("a device named '{}' already exists in regroup", device_name)
                }
                NameConflict::DeviceFileOnDisk => {
                    "a device file already exists on disk".to_string()
                }
                NameConflict::SshConfigBlock => {
                    "the name is already in ~/.ssh/config".to_string()
                }
            }
        })
        .collect();
    
    if messages.len() == 1 {
        messages[0].clone()
    } else {
        format!("{}", messages.join(", "))
    }
}

/// Per-run context shared by every branch of the interactive loop.
pub struct Context<'a> {
    pub home: &'a str,
    pub ssh_config: &'a Path,
    pub dry_run: bool,
    /// This device's SSH server public key: its unique identifier.
    pub local_public_key: &'a SshPublicKey,
    /// Initial verbose state (-v): extra information in the activity log;
    /// the user can toggle it at runtime with 'v'.
    pub verbose: bool,
    /// Force CLI mode (--cli): disable TUI and show all subprocess output.
    pub cli_mode: bool,
    /// This device's hostname, for the verbose identity line.
    pub local_name: &'a str,
    /// This device's IP, for the verbose identity line.
    pub local_ip: Option<&'a str>,
}

/// Run the interactive loop until the user quits or the listener thread dies.
///
/// `beacons` carries devices announced by other devices (already
/// signature-verified and age-checked by the listener). Returns the number
/// of devices accepted this run.
pub fn run(ctx: &Context, udp_port: u16, beacons: &Receiver<BeaconDevice>) -> usize {
    let mut tui = ui::Ui::start(ctx.cli_mode);
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
                "  . this device: {who} (NO SSH SERVER - not announcing)"
            ));
        } else {
            let key = ctx.local_public_key.as_str();
            ui::log(&format!(
                "  . this device: {who} (SSH key: {})",
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

        if let Some(action) = tui.poll_action(POLL_INTERVAL)
            && state.handle_action(ctx, &mut tui, action)
        {
            break;
        }

        // Sync navigation state from TtyUi back to state (navigation keys
        // update the TtyUi directly and return no action)
        if let Some(nav) = tui.navigation() {
            if nav != state.navigation {
                state.navigation = nav;
                state.active_pane = nav.active_pane();
            }
        }

        // The display always reflects the state after processing.
        let plain = !matches!(tui, ui::Ui::Tty(_));
        tui.set_question(state.current_question(ctx, plain));
        let detected = state.detected_view();
        let added = state.added_view();
        
        // Initialize added_selected if not set and there are added devices
        if state.added_selected.is_none() && !added.is_empty() {
            state.added_selected = Some(added[0].name.clone());
        }
        
        // The Users panel follows the machine selected in the Added panel.
        // It only exists while the Added (or Users) pane is on screen.
        let users = if matches!(state.active_pane, ActivePane::Added(_) | ActivePane::Users(_)) {
            state.users_view(ctx)
        } else {
            Vec::new()
        };
        // A user is selected only while the Users pane is active (the
        // selection is refined by navigating into the sub-panel); leaving
        // the sub-panel drops it. Entering it selects the first user
        // automatically — same behavior as the machine panels. A selection
        // that no longer exists (machine changed or deleted, temporary user
        // removed) is dropped the same way.
        if !matches!(state.active_pane, ActivePane::Users(_)) {
            state.users_selected = None;
        } else {
            if state
                .users_selected
                .as_deref()
                .is_some_and(|name| !users.iter().any(|u| u.name == name))
            {
                state.users_selected = None;
            }
            if state.users_selected.is_none() && !users.is_empty() {
                state.users_selected = Some(users[0].name.clone());
            }
            // The Users pane bindings follow the selected user's status:
            // machine+user actions only appear for a confirmed user.
            let confirmed = users
                .iter()
                .position(|u| Some(&u.name) == state.users_selected.as_ref())
                .is_some_and(|i| users[i].status != UserStatus::Temporary);
            state.active_pane = ActivePane::users(confirmed);
        }
        let users_selected = state
            .users_selected
            .as_ref()
            .and_then(|name| users.iter().position(|u| &u.name == name));
        
        let selected = state
            .pending_selected
            .as_ref()
            .and_then(|key| detected.iter().position(|d| d.key == key.as_str()));
        let delete_selected = state
            .delete_selection
            .as_ref()
            .and_then(|name| added.iter().position(|d| &d.name == name));
        let added_selected = state
            .added_selected
            .as_ref()
            .and_then(|name| added.iter().position(|d| &d.name == name));
        
        // When a pane is not active, its selection must be empty
        let selected = match state.active_pane {
            ActivePane::Detected(_) => selected,
            _ => None,
        };
        // Multi-level selection: the parent (the machine) stays selected
        // while the selection is refined on one of its users in the Users
        // pane.
        let added_selected = match state.active_pane {
            ActivePane::Added(_) | ActivePane::Users(_) => added_selected,
            _ => None,
        };
        
        tui.set_devices(detected, added);
        tui.set_selected(selected);
        tui.set_delete_selected(delete_selected);
        tui.set_added_selected(added_selected);
        tui.set_users(users, users_selected);
        tui.set_active_pane(state.active_pane);
        tui.set_navigation(state.navigation);
        tui.set_verbose(state.verbose);
        tui.draw();
    }
    state.new_count
}

/// All mutable state of the interactive loop.
pub struct InteractiveState {
    /// SSH public keys of accepted devices.
    known_keys: HashSet<SshPublicKey>,
    /// Device info by SSH public key for known devices.
    key_to_device: HashMap<SshPublicKey, Device>,
    /// Pinned Ed25519 signing keys (feature D): devices are keyed by their
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
    /// Which pane is currently active in the UI.
    active_pane: ActivePane,
    /// Navigation state: focus + depth in the context hierarchy.
    navigation: Navigation,
    /// Device selected in the Detected panel (full-screen ui), by public
    /// key. None when the queue is empty.
    pending_selected: Option<SshPublicKey>,
    /// Device selected in the Added panel (full-screen ui), by name.
    added_selected: Option<String>,
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
    /// Temporary user per device (device name → user name), added this
    /// run from the Users panel. In memory only: a device's users/ directory
    /// is written by the device itself. Its only purpose is to build the
    /// git remote URL (and thus allow a sync) until the machine's declared
    /// users arrive through that sync.
    temp_users: HashMap<String, String>,
    /// User selected in the Users panel, by name.
    users_selected: Option<String>,
    /// Machine waiting for the user to type a temporary user name.
    temp_user_pending: Option<String>,
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
            key_to_device: HashMap::new(),
            pinned_signing_keys: HashMap::new(),
            rejected_signing_keys: HashSet::new(),
            rotation_pending: None,
            pending_devices: VecDeque::new(),
            pending_keys: HashSet::new(),
            active_pane: ActivePane::main_menu(),
            navigation: Navigation::overview(),
            pending_selected: None,
            added_selected: None,
            known_ip: HashMap::new(),
            rejected_keys: HashSet::new(),
            rename_state: None,
            delete_selection: None,
            wipe_pending: false,
            setup_cd_pending: false,
            temp_users: HashMap::new(),
            users_selected: None,
            temp_user_pending: None,
            verbose: ctx.verbose,
            new_count: 0,
        };
        state.reload(ctx);
        state
    }

    /// Rebuild the lookup maps from the device files on disk (e.g. after a
    /// git sync pulled new device files).
    ///
    /// Devices that disappeared from the repository were removed on
    /// another device: their local traces (~/.ssh/config block, git remote,
    /// last known_hosts IP) are cleaned up so the deletion propagates.
    fn reload(&mut self, ctx: &Context) {
        let old = std::mem::take(&mut self.key_to_device);
        let conf = load_config();
        self.known_keys = conf
            .devices
            .iter()
            .map(|m| m.ssh_public_key.clone())
            .collect();
        self.key_to_device = conf
            .devices
            .iter()
            .map(|m| (m.ssh_public_key.clone(), m.clone()))
            .collect();
        self.pinned_signing_keys = conf
            .devices
            .iter()
            .map(|m| (m.ssh_public_key.clone(), m.signing_public_key.clone()))
            .collect();
        for (key, device) in old {
            if !self.key_to_device.contains_key(&key) {
                let last_ip = self.known_ip.get(&key).cloned();
                remove_local_traces(ctx.home, ctx.ssh_config, &device, last_ip.as_deref());
                ui::log(&format!(
                    "  - {} was removed from the configuration (synced) — local traces cleaned",
                    device.name
                ));
                self.known_ip.remove(&key);
            }
        }
    }

    /// The question to display, derived from the current state. Dry run
    /// never asks anything: pending devices are only logged. In plain mode
    /// the pending queue is asked one device at a time; the full-screen ui
    /// adds devices from the Detected panel instead.
    fn current_question(&self, ctx: &Context, plain: bool) -> Question {
        if self.wipe_pending {
            Question::wipe(self.key_to_device.len())
        } else if self.setup_cd_pending {
            Question::setup_cd(
                shell_rc_file(ctx.home)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        } else if let Some(name) = &self.delete_selection {
            Question::delete(name.clone())
        } else if let Some((device, suggested)) = &self.rename_state {
            Question::rename(device.name.clone(), suggested.clone())
        } else if let Some(device) = &self.rotation_pending {
            Question::trust_key(device.name.clone())
        } else if let Some(device) = &self.temp_user_pending {
            Question::add_temp_user(device.clone())
        } else if plain
            && !ctx.dry_run
            && let Some(device) = self.pending_devices.front()
        {
            Question::add_device(device.name.clone(), device.ip.clone())
        } else {
            Question::idle()
        }
    }

    /// The devices to show in the Detected panel: announced devices
    /// waiting for a decision, sorted by name.
    fn detected_view(&self) -> Vec<UiDevice> {
        let mut devices: Vec<UiDevice> = self
            .pending_devices
            .iter()
            .map(|d| UiDevice {
                key: d.ssh_public_key.as_str().to_string(),
                name: d.name.clone(),
                ip: Some(d.ip.clone()),
                port: d.port,
            })
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }

    /// The devices to show in the Added panel: known devices with their
    /// last known IP, sorted by name.
    fn added_view(&self) -> Vec<UiDevice> {
        let mut devices: Vec<UiDevice> = self
            .key_to_device
            .values()
            .map(|m| UiDevice {
                key: m.ssh_public_key.as_str().to_string(),
                name: m.name.as_str().to_string(),
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
        // The SSH public key is the unique identifier for a device.
        // We cannot use hostname (Termux changes it) or IP (network changes).
        if device.ssh_public_key == *ctx.local_public_key {
            return true;
        }

        // Every announcement from another device pulses its row, so the
        // user sees the app receiving — including silent beacons from known
        // devices with an unchanged IP.
        ui::pulse_device(&device.ssh_public_key);

        // Signing key pinning: a beacon from a known device must be
        // signed with the key pinned when it was accepted.
        if let Some(pinned) = self.pinned_signing_keys.get(&device.ssh_public_key)
            && pinned.as_str() != device.signing_public_key.as_str()
        {
            // Either the device was reinstalled (regroup
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
            // device's key (spoofing/MITM): keep the last verified IP.
            if let Some(stored_device) = self.key_to_device.get(&device.ssh_public_key)
                && !ctx.dry_run
            {
                let verified = match public_key::compute_fingerprint_from_public_key(
                    stored_device.ssh_public_key.as_str(),
                ) {
                    Some(expected) => match public_key::verify_and_add_to_known_hosts(
                        &device.ip,
                        stored_device.port,
                        &expected,
                    ) {
                        public_key::HostKeyVerification::Success => true,
                        public_key::HostKeyVerification::HostUnreachable => {
                            ui::log_device(
                                &stored_device.ssh_public_key,
                                stored_device.name.as_str(),
                                Level::Warning,
                                "unreachable — keeping last verified IP",
                                Some(&format!(
                                    "[~] Known device: {} at {} — unreachable, keeping last verified IP",
                                    stored_device.name.as_str(), device.ip
                                )),
                            );
                            false
                        }
                        public_key::HostKeyVerification::WriteError => {
                            // Verification passed; only the known_hosts write failed
                            ui::log_device(
                                &stored_device.ssh_public_key,
                                stored_device.name.as_str(),
                                Level::Warning,
                                "verified, but known_hosts write failed",
                                Some(&format!(
                                    "WARNING: could not write {} to ~/.ssh/known_hosts",
                                    stored_device.name.as_str()
                                )),
                            );
                            true
                        }
                        public_key::HostKeyVerification::FingerprintMismatch {
                            expected,
                            actual,
                        } => {
                            ui::log_device(
                                &stored_device.ssh_public_key,
                                stored_device.name.as_str(),
                                Level::Security,
                                "host key MISMATCH — possible spoofing/MITM",
                                Some(&format!(
                                    "[!] SECURITY: {} announces IP {} but serves a DIFFERENT host key — possible spoofing/MITM. SSH config NOT updated.\n    expected fingerprint: {}\n    actual fingerprint:   {}",
                                    stored_device.name.as_str(), device.ip, expected, actual
                                )),
                            );
                            false
                        }
                    },
                    None => {
                        ui::log_device(
                            &device.ssh_public_key,
                            device.name.as_str(),
                            Level::Warning,
                            "cannot compute fingerprint — keeping last verified IP",
                            Some(&format!(
                                "WARNING: cannot compute fingerprint of {}'s host key — keeping last verified IP",
                                device.name.as_str()
                            )),
                        );
                        false
                    }
                };
                if verified {
                    let old_ip = self.known_ip.get(&device.ssh_public_key).cloned();
                    let _ = update_block(
                        ctx.ssh_config,
                        stored_device.name.as_str(),
                        &device.ip,
                        stored_device.port,
                        stored_device.port_explicit,
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
                            &device.ssh_public_key,
                            device.name.as_str(),
                            Level::Info,
                            "cleaned old known_hosts entries",
                            Some(&format!(
                                "  ~ Cleaned old known_hosts entries for {old_ip}"
                            )),
                        );
                    }
                    ui::log_device(
                        &device.ssh_public_key,
                        device.name.as_str(),
                        Level::Success,
                        "IP updated — host key verified",
                        Some(&format!(
                            "[~] Known device: {} at {} (IP updated, host key verified)",
                            device.name.as_str(), device.ip
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
                crate::interactive::utils::key_preview(&device.ssh_public_key)
            ));
            return false;
        }
        self.pending_devices.push_back(device);
        // Select the new device so it can be added right away.
        if self.pending_selected.is_none() {
            self.pending_selected = Some(self.pending_devices.back().unwrap().ssh_public_key.clone());
        }
        let added_device = self.pending_devices.back().unwrap();
        ui::log_device(
            &added_device.ssh_public_key,
            added_device.name.as_str(),
            Level::Info,
            "new device detected",
            Some(&format!(
                "[!] New device: {} ({}) - key: {}",
                added_device.name.as_str(),
                added_device.ip,
                crate::interactive::utils::key_preview(&added_device.ssh_public_key)
            )),
        );
        false
    }

    /// Apply a user decision. Returns true when the user asked to quit.
    fn handle_action(&mut self, ctx: &Context, tui: &mut ui::Ui, action: Action) -> bool {
        match action {
            Action::Quit => return true,
            Action::BackToMainMenu => {
                self.navigation = Navigation::overview();
                self.active_pane = ActivePane::main_menu();
                self.pending_selected = None;
                self.added_selected = None;
                return false;
            }
            Action::Sync => {
                // Dry run must not modify anything: skip git sync
                if ctx.dry_run {
                    ui::log("  (dry run - git sync skipped)");
                } else {
                    tui.with_clean_terminal(|| {
                        sync_git(ctx.home, self.verbose, &self.temp_users);
                        // A pull may have brought new device configs and
                        // the machine-declared users: reload
                        self.reload(ctx);
                    });
                }
            }
            Action::Add => self.accept_selected(ctx, tui),
            Action::Ignore => self.ignore_selected(),
            Action::AddCurrent => self.accept_front(ctx, tui),
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
                    tui.with_clean_terminal(|| {
                        trust_new_signing_key(ctx.home, &device);
                    });
                    ui::log_device(
                        &device.ssh_public_key,
                        device.name.as_str(),
                        Level::Success,
                        "new signing key trusted",
                        Some(&format!(
                            "  + New signing key for {} trusted — its next beacon will be processed normally.",
                            device.name.as_str()
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
                        device.name.as_str(),
                        Level::Warning,
                        "new signing key rejected",
                        Some(&format!(
                            "  New signing key for {} rejected — its beacons are ignored.",
                            device.name.as_str()
                        )),
                    );
                }
            }
            Action::Rename(name) => self.rename(ctx, tui, name),
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
            Action::PendingSelectPrev => {
                self.move_pending_selection(false);
            }
            Action::PendingSelectNext => {
                self.move_pending_selection(true);
            }
            Action::AddedSelectPrev => {
                self.move_added_selection(false);
            }
            Action::AddedSelectNext => {
                self.move_added_selection(true);
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
            Action::SwitchToDetected => {
                self.navigation = Navigation { focus: NavFocus::Detected, depth: Some(NavDepth::Fullscreen) };
                self.active_pane = ActivePane::detected();
                self.added_selected = None;
                // Initialize pending_selected if there are pending devices
                if self.pending_selected.is_none() && !self.pending_devices.is_empty() {
                    self.pending_selected = Some(self.pending_devices[0].ssh_public_key.clone());
                }
            }
            Action::SwitchToAdded => {
                self.navigation = Navigation { focus: NavFocus::Added, depth: Some(NavDepth::Fullscreen) };
                self.active_pane = ActivePane::added();
                self.pending_selected = None;
            }
            Action::SwitchToUsers => {
                // Only reachable from the Added pane with a machine
                // selected (the tty gates the key). No user is selected
                // yet: the display loop selects the first one.
                self.navigation = Navigation { focus: NavFocus::Users, depth: Some(NavDepth::Fullscreen) };
                self.active_pane = ActivePane::users(false);
                self.users_selected = None;
            }
            Action::BackToAdded => {
                // Leaving the sub-panel drops the user selection; the
                // machine stays selected.
                self.navigation = Navigation { focus: NavFocus::Added, depth: Some(NavDepth::Fullscreen) };
                self.active_pane = ActivePane::added();
                self.users_selected = None;
            }
            Action::UsersSelectPrev => {
                self.move_users_selection(ctx, false);
            }
            Action::UsersSelectNext => {
                self.move_users_selection(ctx, true);
            }
            Action::AddTempUser => {
                if let Some(name) = self.added_selected.clone() {
                    self.temp_user_pending = Some(name);
                }
            }
            Action::TempUserName(name) => {
                self.add_temp_user(name);
            }
            Action::CancelTempUser => {
                self.temp_user_pending = None;
            }
            Action::RemoveTempUser => {
                self.remove_temp_user();
            }
            Action::SwitchToLogs => {
                self.navigation = Navigation { focus: NavFocus::Logs, depth: Some(NavDepth::Fullscreen) };
                self.active_pane = ActivePane::logs();
                self.pending_selected = None;
                self.added_selected = None;
            }
            Action::DeleteConfirm => self.delete_selected(ctx, tui),
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
                let devices: Vec<Device> = self.key_to_device.values().cloned().collect();
                tui.with_clean_terminal(|| {
                    wipe_config(ctx.home, ctx.ssh_config, &devices);
                });
                // Drop the wiped devices from memory, removing their
                // known_hosts entries (last known IP) on the way — wipe
                // itself leaves known_hosts alone (see its doc comment).
                for (key, _) in std::mem::take(&mut self.key_to_device) {
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
            Action::InstallKeyToSelected => self.install_key_to_selected(ctx, tui),
            Action::RemoveKeyFromSelected => self.remove_key_from_selected(ctx, tui),
            Action::DeleteSelected => self.delete_selected_device(ctx, tui),
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
            .key_to_device
            .values()
            .map(|m| m.name.as_str().to_string())
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

    /// Move the selection of the Added panel to the next (or previous
    /// when `forward` is false) known device, in name order. With no
    /// selection, selects the first one.
    fn move_added_selection(&mut self, forward: bool) {
        let names = self.deletable_names();
        if names.is_empty() {
            self.added_selected = None;
            return;
        }
        let pos = names
            .iter()
            .position(|n| Some(n) == self.added_selected.as_ref());
        let new_pos = match pos {
            Some(p) if forward => (p + 1).min(names.len() - 1),
            Some(p) => p.saturating_sub(1),
            None => 0,
        };
        self.added_selected = Some(names[new_pos].clone());
        self.users_selected = None;
    }

    /// Move the selection of the Detected panel to the next (or previous
    /// when `forward` is false) pending device, in name order. With no
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

    /// Point the Detected-panel selection at a sensible device after the
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
    fn delete_selected(&mut self, ctx: &Context, tui: &mut ui::Ui) {
        let Some(name) = self.delete_selection.take() else {
            return;
        };
        let Some(device) = self
            .key_to_device
            .values()
            .find(|m| m.name.as_str() == name)
            .cloned()
        else {
            return;
        };
        let key = device.ssh_public_key.clone();
        let last_ip = self.known_ip.get(&key).cloned();
        if tui.with_clean_terminal(|| {
            remove_device(ctx.home, ctx.ssh_config, &device, last_ip.as_deref())
        }) {
            ui::log(&format!(
                "  - Removed {} (device file, ~/.ssh/config block, git remote)",
                device.name
            ));
        } else {
            ui::log(&format!(
                "  ! Could not fully remove {} — see warnings",
                device.name
            ));
        }
        self.known_keys.remove(&key);
        self.key_to_device.remove(&key);
        self.pinned_signing_keys.remove(&key);
        self.known_ip.remove(&key);
    }

    /// Add the device selected in the Detected panel. A name collision
    /// redirects to the rename question instead.
    fn accept_selected(&mut self, ctx: &Context, tui: &mut ui::Ui) {
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
        self.accept_device(ctx, tui, device);
    }

    /// Add the front of the pending queue (plain mode). A name collision
    /// redirects to the rename question instead.
    fn accept_front(&mut self, ctx: &Context, tui: &mut ui::Ui) {
        let Some(device) = self.pending_devices.pop_front() else {
            return;
        };
        self.pending_keys.remove(&device.ssh_public_key);
        self.after_selection_change();
        self.accept_device(ctx, tui, device);
    }

    /// Accept `device`: a name collision redirects to the rename question.
    fn accept_device(&mut self, ctx: &Context, tui: &mut ui::Ui, device: BeaconDevice) {
        // If the name is already used by another device,
        // ask the user to pick a new one.
        if device_name_taken(&device.name, ctx.home, ctx.ssh_config) {
            let suggested = unique_device_name(&device.name, ctx.home, ctx.ssh_config);
            let conflicts = find_name_conflicts(&device.name, ctx.home, ctx.ssh_config);
            let conflict_msg = format_name_conflicts(&conflicts);
            ui::log(&format!(
                "  ! Name '{}' cannot be used: {}",
                device.name, conflict_msg
            ));
            self.rename_state = Some((device, suggested));
            return;
        }
        let name = device.name.clone();
        self.add_device(ctx, tui, device, name);
    }

    /// Ignore the device selected in the Detected panel: it is dropped
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
    fn rename(&mut self, ctx: &Context, tui: &mut ui::Ui, name: String) {
        let Some((device, _)) = self.rename_state.take() else {
            return;
        };
        if device_name_taken(&name, ctx.home, ctx.ssh_config) {
            let suggested = unique_device_name(&name, ctx.home, ctx.ssh_config);
            let conflicts = find_name_conflicts(&name, ctx.home, ctx.ssh_config);
            let conflict_msg = format_name_conflicts(&conflicts);
            ui::log(&format!(
                "  ! Name '{}' cannot be used: {}",
                name, conflict_msg
            ));
            self.rename_state = Some((device, suggested));
            return;
        }
        self.add_device(ctx, tui, device, name);
    }

    /// Add `device` under `name`: write to disk and update the in-memory
    /// maps so future beacons for it are treated as a known device.
    fn add_device(&mut self, ctx: &Context, tui: &mut ui::Ui, beacon_device: BeaconDevice, name: String) {
        if let Some(device) = tui.with_clean_terminal(|| {
            add_device_to_config(
                ctx.home,
                ctx.ssh_config,
                &beacon_device,
                &name,
                &mut self.new_count,
            )
        }) {
            // Pin the signing key the device used when accepted
            self.pinned_signing_keys.insert(
                beacon_device.ssh_public_key.clone(),
                beacon_device.signing_public_key.clone(),
            );
            self.known_keys.insert(beacon_device.ssh_public_key.clone());
            self.key_to_device
                .insert(beacon_device.ssh_public_key.clone(), device);
            self.known_ip
                .insert(beacon_device.ssh_public_key.clone(), beacon_device.ip.clone());
        }
    }
    
    /// The users of the machine selected in the Added panel, for the Users
    /// panel: the users the machine declared in its configuration (users/,
    /// arrived through git sync), plus the temporary user if one was added
    /// this run and the sync has not brought a user of the same name yet.
    /// A confirmed user the local user has an outgoing connection to shows
    /// as "SSH key installed".
    fn users_view(&self, ctx: &Context) -> Vec<UiUser> {
        let Some(name) = self.added_selected.as_ref() else {
            return Vec::new();
        };
        let config_dir = Path::new(ctx.home).join(CONFIG_DIR).join(GIT_DIR);
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        let connections_dir = config_dir
            .join("devices")
            .join(ctx.local_name)
            .join("connections");
        let mut users = Vec::new();
        if let Ok(known) = load_device_users(&config_dir, name) {
            for user in known {
                // Connection file names sanitize their members (see
                // config::connection), so mirror that here.
                let sanitize = |s: &str| s.replace('@', "_").replace('/', "_").replace(' ', "_");
                let connection_file = format!(
                    "{}->{}@{}.json",
                    sanitize(&current_user),
                    sanitize(&user.name),
                    sanitize(name)
                );
                let installed = connections_dir.join(connection_file).exists();
                users.push(UiUser {
                    name: user.name,
                    status: if installed {
                        UserStatus::KeyInstalled
                    } else {
                        UserStatus::Confirmed
                    },
                });
            }
        }
        // A temporary user is superseded by a declared user of the same
        // name: the sync brought the machine's own data.
        if let Some(temp) = self.temp_users.get(name)
            && !users.iter().any(|u| u.name == *temp)
        {
            users.push(UiUser {
                name: temp.clone(),
                status: UserStatus::Temporary,
            });
        }
        users
    }

    /// Whether `user` is the temporary user of `device`.
    fn is_temporary_user(&self, device: &str, user: &str) -> bool {
        self.temp_users
            .get(device)
            .is_some_and(|temp| temp == user)
    }

    /// Move the Users panel selection to the next (or previous when
    /// `forward` is false) user of the selected machine, in the same order
    /// as the Users panel. With no selection, selects the first one.
    fn move_users_selection(&mut self, ctx: &Context, forward: bool) {
        let names: Vec<String> = self
            .users_view(ctx)
            .iter()
            .map(|u| u.name.clone())
            .collect();
        if names.is_empty() {
            self.users_selected = None;
            return;
        }
        let pos = names
            .iter()
            .position(|n| Some(n) == self.users_selected.as_ref());
        let new_pos = match pos {
            Some(p) if forward => (p + 1).min(names.len() - 1),
            Some(p) => p.saturating_sub(1),
            None => 0,
        };
        self.users_selected = Some(names[new_pos].clone());
    }

    /// The user typed a temporary user name for the pending machine:
    /// remember it (in memory only) and select it, so a git sync can reach
    /// the machine. The sync replaces it with the machine's declared users.
    fn add_temp_user(&mut self, name: String) {
        let Some(device) = self.temp_user_pending.take() else {
            return;
        };
        let name = name.trim().to_string();
        if name.is_empty() || name.contains('@') || name.contains('/') || name.contains(' ') {
            ui::log("  ! A user name must be non-empty and contain no '@', '/' or space");
            return;
        }
        ui::log(&format!(
            "  + Temporary user {name} on {device} (in memory only) — press 's' to sync and get the machine's declared users"
        ));
        self.temp_users.insert(device.clone(), name.clone());
        self.users_selected = Some(name);
    }

    /// Remove the selected temporary user of the selected machine.
    fn remove_temp_user(&mut self) {
        let (Some(device), Some(user)) = (self.added_selected.clone(), self.users_selected.clone())
        else {
            return;
        };
        if self.is_temporary_user(&device, &user) {
            self.temp_users.remove(&device);
            self.users_selected = None;
            ui::log(&format!("  - Temporary user {user} on {device} removed"));
        }
    }

    /// Install the local user's SSH key towards the user selected in the
    /// Users panel, on the machine selected in the Added panel. A machine
    /// action alone is not enough: this is a machine+user action.
    fn install_key_to_selected(&mut self, ctx: &Context, ui: &mut ui::Ui) {
        let (Some(target_name), Some(target_user)) =
            (self.added_selected.clone(), self.users_selected.clone())
        else {
            ui::log("Select a machine, then one of its users (Right) first");
            return;
        };
        if self.is_temporary_user(&target_name, &target_user) {
            ui::log(&format!(
                "  ! {target_user} on {target_name} is temporary — sync git ('s') to get the machine's declared users first"
            ));
            return;
        }
        let Some(target_device) = self
            .key_to_device
            .values()
            .find(|m| m.name.as_str() == target_name)
            .cloned()
        else {
            ui::log(&format!("Machine '{}' not found", target_name));
            return;
        };

        let config_dir = Path::new(ctx.home).join(CONFIG_DIR).join(GIT_DIR);
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let target_port = target_device.port;

        ui::log(&format!("Installing key to {}@{}", target_user, target_name));

        let result = ui.with_clean_terminal(|| {
            crate::tui::key_actions::install_key_action(
                ctx.home,
                &config_dir,
                ctx.local_name,
                &current_user,
                &target_name,
                &target_user,
                target_port,
            )
        });

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

    /// Remove the SSH key connection towards the user selected in the
    /// Users panel, on the machine selected in the Added panel.
    fn remove_key_from_selected(&mut self, ctx: &Context, ui: &mut ui::Ui) {
        let (Some(target_name), Some(target_user)) =
            (self.added_selected.clone(), self.users_selected.clone())
        else {
            ui::log("Select a machine, then one of its users (Right) first");
            return;
        };
        if self.is_temporary_user(&target_name, &target_user) {
            ui::log(&format!(
                "  ! {target_user} on {target_name} is temporary — no SSH key connection exists"
            ));
            return;
        }
        let config_dir = Path::new(ctx.home).join(CONFIG_DIR).join(GIT_DIR);
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());

        let connection_id = format!("{}->{}@{}", current_user, target_user, target_name);
        let target_port = self
            .key_to_device
            .values()
            .find(|m| m.name.as_str() == target_name)
            .map(|m| m.port)
            .unwrap_or(22);

        ui::log(&format!("Removing connection: {}", connection_id));

        let result = ui.with_clean_terminal(|| {
            crate::tui::key_actions::remove_key_action(
                ctx.home,
                &config_dir,
                ctx.local_name,
                &current_user,
                &current_user,
                &target_name,
                &target_user,
                target_port,
                &connection_id,
            )
        });

        match result {
            Ok(crate::tui::key_actions::KeyActionResult::Success(msg)) => {
                ui::log(&format!("Success: {}", msg));
            }
            Ok(crate::tui::key_actions::KeyActionResult::Error(msg)) => {
                ui::log(&format!("Error: {}", msg));
            }
            Ok(crate::tui::key_actions::KeyActionResult::Cancelled) => {
                ui::log("Removal cancelled");
            }
            Err(e) => {
                ui::log(&format!("Error: {}", e));
            }
        }
    }

    /// Delete the currently selected device from Added pane
    fn delete_selected_device(&mut self, ctx: &Context, tui: &mut ui::Ui) {
        let Some(name) = self.added_selected.clone() else {
            ui::log("No device selected in Added pane");
            return;
        };
        
        let Some(device) = self.key_to_device.values().find(|m| m.name.as_str() == name).cloned() else {
            ui::log(&format!("Device '{}' not found", name));
            return;
        };
        
        let key = device.ssh_public_key.clone();
        let last_ip = self.known_ip.get(&key).cloned();
        
        if tui.with_clean_terminal(|| {
            remove_device(ctx.home, ctx.ssh_config, &device, last_ip.as_deref())
        }) {
            ui::log(&format!("  - Removed {} (device file, ~/.ssh/config block, git remote)", device.name));
        } else {
            ui::log(&format!("  ! Could not fully remove {} — see warnings", device.name));
        }
        
        self.known_keys.remove(&key);
        self.key_to_device.remove(&key);
        self.pinned_signing_keys.remove(&key);
        self.known_ip.remove(&key);
        self.temp_users.remove(name.as_str());
        self.added_selected = None;
        self.users_selected = None;
    }
}

