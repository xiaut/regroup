//! UDP beacon module
//!
//! This module handles UDP beacon announcement and listening for device discovery.
//!
//! Beacon format (tab-separated):
//! name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64
//!
//! The signed message is: "name|ssh_public_key|user|ip|port|timestamp"
//!
//! # Machine Identification
//!
//! **CRITICAL**: Machines are uniquely identified by their SSH public key, NOT by hostname or IP.
//! - Hostname can change (especially in Termux where it may be randomized on each launch)
//! - IP addresses can change with network changes (WiFi, VPN, DHCP, etc.)
//! - SSH public key is persistent and uniquely identifies a machine
//!
//! This is why self-detection filtering MUST use the SSH public key to determine
//! "this beacon is from me" rather than comparing hostname or IP address.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use super::NetworkInterface;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};

/// Represents a device announced via UDP beacon
#[derive(Clone, Debug)]
pub struct BeaconDevice {
    pub name: String,
    pub signing_public_key: SigningPublicKeyB64,
    pub ssh_public_key: SshPublicKey,
    pub user: String,
    pub ip: String,
    pub port: u16,
}

/// What a device announces about itself in every beacon: its name, its SSH
/// server's public key (its unique identity on the network), the SSH username,
/// the IP and port its SSH server listens on, and the Ed25519 key beacons are signed with.
#[derive(Clone)]
pub struct BeaconIdentity {
    pub name: String,
    pub ssh_public_key: SshPublicKey,
    pub user: String,
    pub ip: String,
    pub ssh_port: u16,
    pub signing_key: ed25519_dalek::SigningKey,
}

/// Broadcasts signed beacon message every 2 s until the deadline on all local interfaces.
/// Runs in its own thread, in parallel with the continuous listener, for the same window.
/// Each interface sends to its own broadcast address.
///
/// If `running` is given, the loop stops early when the flag becomes false
/// (checked every ~100 ms so shutdown is responsive).
///
/// The beacon format is: name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64
/// The signed message is: "name|ssh_public_key|user|ip|port|timestamp"
pub fn announce_beacon(
    identity: &BeaconIdentity,
    port: u16,
    secs: u64,
    interfaces: &[NetworkInterface],
    running: Option<&AtomicBool>,
) {
    use crate::signing;

    if interfaces.is_empty() {
        crate::ui::log("WARNING: no local interfaces found — announcing disabled");
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(secs);
    let timestamp = chrono_timestamp();

    // Create one socket per interface to send on the correct broadcast address
    let mut sockets = Vec::new();
    for iface in interfaces {
        if let Ok(sock) = std::net::UdpSocket::bind(format!("{}:0", iface.ip))
            && sock.set_broadcast(true).is_ok()
        {
            sockets.push((sock, iface.broadcast.clone(), iface.ip.clone()));
        }
    }

    if sockets.is_empty() {
        crate::ui::log("WARNING: cannot bind to any interface for announcing");
        return;
    }

    // Create the signed beacon message
    let beacon_message = match signing::sign_beacon(
        &identity.name,
        &identity.ssh_public_key,
        &identity.user,
        &identity.ip,
        identity.ssh_port,
        &timestamp,
        &identity.signing_key,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            crate::ui::log(&format!("WARNING: cannot sign beacon: {}", e));
            return;
        }
    };

    let mut last_send: Option<Instant> = None;
    while Instant::now() < deadline && running.is_none_or(|r| r.load(Ordering::Relaxed)) {
        if last_send.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            last_send = Some(Instant::now());
            for (sock, broadcast, _) in &sockets {
                let target = format!("{}:{}", broadcast, port);
                if let Ok(addr) = target.parse::<SocketAddr>() {
                    let _ = sock.send_to(beacon_message.as_bytes(), addr);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Generate a timestamp string for beacon messages
fn chrono_timestamp() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

/// Maximum age for beacon messages (5 minutes = 300 seconds)
const MAX_BEACON_AGE: u64 = 300;

/// Maximum tolerated clock skew for beacon timestamps (seconds in the future)
const MAX_BEACON_FUTURE: u64 = 60;

/// Listens for beacons continuously and sends each via channel.
/// Runs until the `running` flag becomes false or the channel is disconnected.
///
/// Only beacons with valid signatures are forwarded via the channel.
/// Beacons from the local machine (self-detection) are filtered out based on SSH public key.
/// The SSH public key is the unique identifier for a machine - this is what defines
/// "this is me" and not name or IP (which can change).
pub fn listen_beacons_continuous(
    port: u16,
    sender: std::sync::mpsc::Sender<BeaconDevice>,
    local_ssh_public_key: SshPublicKey,
    running: Arc<AtomicBool>,
) {
    use crate::signing;

    let sock = match std::net::UdpSocket::bind(("0.0.0.0", port)) {
        Ok(s) => s,
        Err(e) => {
            crate::ui::log(&format!(
                "WARNING: cannot listen on UDP port {} ({}) — beacons ignored",
                port, e
            ));
            return;
        }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let mut buf = [0u8; 4096];

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let parts: Vec<&str> = text.split('\t').collect();

                // Expected format: name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64
                if parts.len() != 8 {
                    continue; // Invalid format
                }

                let ip = parts[4].to_string();
                let timestamp: u64 = parts[6].parse().unwrap_or(0);

                // Check beacon age
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                if now.saturating_sub(timestamp) > MAX_BEACON_AGE {
                    crate::ui::log(&format!(
                        "WARNING: received expired beacon from {} (age: {}s)",
                        ip,
                        now.saturating_sub(timestamp)
                    ));
                    continue;
                }

                // A timestamp in the future would bypass the age check
                // forever (saturating_sub yields 0): allow only a small
                // clock skew between machines.
                if timestamp > now + MAX_BEACON_FUTURE {
                    crate::ui::log(&format!(
                        "WARNING: received beacon from {} with a future timestamp ({}s ahead) — rejected",
                        ip,
                        timestamp - now
                    ));
                    continue;
                }

                // Verify the signature
                let beacon_str = parts.join("\t");
                match signing::verify_beacon(&beacon_str, None) {
                    Ok((name, ssh_public_key, user, ip_verified, port)) => {
                        // Check IP consistency
                        if ip_verified != ip {
                            crate::ui::log(&format!(
                                "WARNING: beacon IP mismatch - claimed: {}, verified: {}",
                                ip, ip_verified
                            ));
                            continue;
                        }

                        // IMPORTANT: Filter out self-detection based on SSH public key only.
                        // The SSH public key is the unique identifier for a machine.
                        // We cannot rely on hostname (changes in Termux) or IP (changes with network).
                        // If the beacon's SSH public key matches our own, this is us - ignore it.
                        if ssh_public_key == local_ssh_public_key {
                            continue;
                        }

                        let device = BeaconDevice {
                            name,
                            signing_public_key: SigningPublicKeyB64::from(parts[1]),
                            ssh_public_key,
                            user,
                            ip,
                            port,
                        };
                        // If the channel is disconnected, the main loop is gone: stop
                        if sender.send(device).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        crate::ui::log(&format!(
                            "WARNING: invalid beacon signature from {}: {}",
                            ip, e
                        ));
                    }
                }
            }
            Err(_) => {
                // Read timeout, sleep briefly to avoid busy loop
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
