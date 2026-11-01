//! UDP beacon module
//!
//! This module handles UDP beacon announcement and listening for device discovery.
//!
//! Beacon format (tab-separated):
//! name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
//!
//! The signed message is: "name|ssh_public_key|ip|port|timestamp"
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::NetworkInterface;
use crate::ssh::keys::{SignatureB64, SigningPublicKeyB64, SshPublicKey};

/// Represents a device announced via UDP beacon
#[derive(Clone, Debug)]
pub struct BeaconDevice {
    pub name: String,
    pub signing_public_key: SigningPublicKeyB64,
    pub ssh_public_key: SshPublicKey,
    pub ip: String,
    pub port: u16,
    pub timestamp: String,
    pub signature: SignatureB64,
}

/// Broadcasts signed beacon message every 2 s until the deadline on all local interfaces.
/// Runs in its own thread, in parallel with listen_beacons, for the same window.
/// Each interface sends to its own broadcast address.
/// 
/// If `running` is given, the loop stops early when the flag becomes false
/// (checked every ~100 ms so shutdown is responsive).
/// 
/// The beacon format is: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
/// The signed message is: "name|ssh_public_key|ip|port|timestamp"
pub fn announce_beacon(
    port: u16,
    secs: u64,
    name: &str,
    ssh_public_key: &SshPublicKey,
    ip: &str,
    port_ssh: u16,
    signing_key: &ed25519_dalek::SigningKey,
    interfaces: &[NetworkInterface],
    running: Option<&AtomicBool>,
) {
    use crate::signing;
    
    if interfaces.is_empty() {
        eprintln!("WARNING: no local interfaces found — announcing disabled");
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(secs);
    let timestamp = chrono_timestamp();

    // Create one socket per interface to send on the correct broadcast address
    let mut sockets = Vec::new();
    for iface in interfaces {
        if let Ok(sock) = std::net::UdpSocket::bind(format!("{}:0", iface.ip)) {
            if sock.set_broadcast(true).is_ok() {
                sockets.push((sock, iface.broadcast.clone(), iface.ip.clone()));
            }
        }
    }

    if sockets.is_empty() {
        eprintln!("WARNING: cannot bind to any interface for announcing");
        return;
    }

    // Create the signed beacon message
    let beacon_message = match signing::sign_beacon(
        name,
        ssh_public_key,
        ip,
        port_ssh,
        &timestamp,
        signing_key,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            eprintln!("WARNING: cannot sign beacon: {}", e);
            return;
        }
    };

    let mut last_send: Option<Instant> = None;
    while Instant::now() < deadline && running.map_or(true, |r| r.load(Ordering::Relaxed)) {
        if last_send.map_or(true, |t| t.elapsed() >= Duration::from_secs(2)) {
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

/// Listens for signed beacon messages during the window. Returns BeaconDevice list,
/// deduplicated by IP. Only beacons with valid signatures are accepted.
/// 
/// Beacons from the local machine (self-detection) are filtered out based on SSH public key.
/// The SSH public key is the unique identifier for a machine - this is what defines
/// "this is me" and not name or IP (which can change).
/// 
/// Expected format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
/// Signed message: "name|ssh_public_key|ip|port|timestamp"
pub fn listen_beacons(port: u16, secs: u64, local_ssh_public_key: &SshPublicKey) -> Vec<BeaconDevice> {
    use crate::signing;
    
    let mut beacons: Vec<BeaconDevice> = Vec::new();
    let sock = match std::net::UdpSocket::bind(("0.0.0.0", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING: cannot listen on UDP port {} ({}) — beacons ignored", port, e);
            return beacons;
        }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut buf = [0u8; 4096];
    
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let parts: Vec<&str> = text.split('\t').collect();
                
                // Expected: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
                if parts.len() != 7 {
                    continue; // Invalid format
                }
                
                let ip = parts[3].to_string();
                let timestamp: u64 = parts[5].parse().unwrap_or(0);
                
                // Check if beacon is too old
                if now.saturating_sub(timestamp) > MAX_BEACON_AGE {
                    eprintln!("WARNING: received expired beacon from {} (age: {}s)", 
                        ip, now.saturating_sub(timestamp));
                    continue;
                }
                
                // Verify the signature
                let beacon_str = parts.join("\t");
                match signing::verify_beacon(&beacon_str, None) {
                    Ok((name, ssh_public_key, ip_verified, port, timestamp_str)) => {
                        // Check IP consistency
                        if ip_verified != ip {
                            eprintln!("WARNING: beacon IP mismatch - claimed: {}, verified: {}", 
                                ip, ip_verified);
                            continue;
                        }
                        
                        // IMPORTANT: Filter out self-detection based on SSH public key only.
                        // The SSH public key is the unique identifier for a machine.
                        // We cannot rely on hostname (changes in Termux) or IP (changes with network).
                        // If the beacon's SSH public key matches our own, this is us - ignore it.
                        if &ssh_public_key == local_ssh_public_key {
                            continue;
                        }
                        
                        // Check if we already have this device by IP
                        if !beacons.iter().any(|b| b.ip == ip) {
                            beacons.push(BeaconDevice {
                                name,
                                signing_public_key: SigningPublicKeyB64::from(parts[1]),
                                ssh_public_key,
                                ip,
                                port,
                                timestamp: timestamp_str,
                                signature: SignatureB64::from(parts[6]),
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("WARNING: invalid beacon signature from {}: {}", ip, e);
                    }
                }
            }
            Err(_) => {} // read timeout: loop until the deadline
        }
    }
    beacons
}

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
            eprintln!("WARNING: cannot listen on UDP port {} ({}) — beacons ignored", port, e);
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
                
                // Expected format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
                if parts.len() != 7 {
                    continue; // Invalid format
                }
                
                let ip = parts[3].to_string();
                let timestamp: u64 = parts[5].parse().unwrap_or(0);
                
                // Check beacon age
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                
                if now.saturating_sub(timestamp) > MAX_BEACON_AGE {
                    eprintln!("WARNING: received expired beacon from {} (age: {}s)", 
                        ip, now.saturating_sub(timestamp));
                    continue;
                }
                
                // Verify the signature
                let beacon_str = parts.join("\t");
                match signing::verify_beacon(&beacon_str, None) {
                    Ok((name, ssh_public_key, ip_verified, port, timestamp_str)) => {
                        // Check IP consistency
                        if ip_verified != ip {
                            eprintln!("WARNING: beacon IP mismatch - claimed: {}, verified: {}", 
                                ip, ip_verified);
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
                            ip,
                            port,
                            timestamp: timestamp_str,
                            signature: SignatureB64::from(parts[6]),
                        };
                        // If the channel is disconnected, the main loop is gone: stop
                        if sender.send(device).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("WARNING: invalid beacon signature from {}: {}", ip, e);
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
