//! UDP beacon module
//!
//! This module handles UDP beacon announcement and listening for device discovery.
//!
//! Beacon format (tab-separated):
//! name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
//!
//! The signed message is: "name|ssh_public_key|ip|port|timestamp"

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use super::NetworkInterface;

/// Represents a device announced via UDP beacon
#[derive(Clone, Debug)]
pub struct BeaconDevice {
    pub name: String,
    pub signing_public_key: String,
    pub ssh_public_key: String,
    pub ip: String,
    pub port: u16,
    pub timestamp: String,
    pub signature: String,
}

/// Broadcasts signed beacon message every 2 s until the deadline on all local interfaces.
/// Runs in its own thread, in parallel with listen_beacons, for the same window.
/// Each interface sends to its own broadcast address.
/// 
/// The beacon format is: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
/// The signed message is: "name|ssh_public_key|ip|port|timestamp"
pub fn announce_beacon(
    port: u16,
    secs: u64,
    name: &str,
    ssh_public_key: &str,
    ip: &str,
    port_ssh: u16,
    signing_key: &ed25519_dalek::SigningKey,
    interfaces: &[NetworkInterface],
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

    while Instant::now() < deadline {
        for (sock, broadcast, _) in &sockets {
            let target = format!("{}:{}", broadcast, port);
            if let Ok(addr) = target.parse::<SocketAddr>() {
                let _ = sock.send_to(beacon_message.as_bytes(), addr);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
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

/// Listens for signed beacon messages during the window. Returns BeaconDevice list,
/// deduplicated by IP. Only beacons with valid signatures are accepted.
/// 
/// Expected format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
/// Signed message: "name|ssh_public_key|ip|port|timestamp"
pub fn listen_beacons(port: u16, secs: u64) -> Vec<BeaconDevice> {
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
    
    // Maximum age for beacon messages (5 minutes = 300 seconds)
    let max_beacon_age = 300u64;
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
                if parts.len() >= 7 {
                    let ip = parts[3].to_string();
                    let _port: u16 = parts[4].parse().unwrap_or(22);
                    let timestamp: u64 = parts[5].parse().unwrap_or(0);
                    
                    // Check if beacon is too old
                    if now.saturating_sub(timestamp) > max_beacon_age {
                        eprintln!("WARNING: received expired beacon from {} (age: {}s)", 
                            ip, now.saturating_sub(timestamp));
                        continue;
                    }
                    
                    // Verify the signature
                    let beacon_str = parts.join("\t");
                    match signing::verify_beacon(&beacon_str, None) {
                        Ok((name, ssh_public_key, ip_verified, verified_port, timestamp_str)) => {
                            // Check IP consistency
                            if ip_verified != ip {
                                eprintln!("WARNING: beacon IP mismatch - claimed: {}, verified: {}", 
                                    ip, ip_verified);
                                continue;
                            }
                            
                            // Check if we already have this device by IP
                            if !beacons.iter().any(|b| b.ip == ip) {
                                beacons.push(BeaconDevice {
                                    name,
                                    signing_public_key: parts[1].to_string(),
                                    ssh_public_key,
                                    ip,
                                    port: verified_port,
                                    timestamp: timestamp_str,
                                    signature: parts[6].to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            eprintln!("WARNING: invalid beacon signature from {}: {}", ip, e);
                        }
                    }
                } else {
                    // Old format (name\tpublic_key\tip) - backward compatibility
                    if parts.len() >= 3 {
                        eprintln!("WARNING: received beacon in old format from {}", parts[2]);
                        // For backward compatibility, accept but mark as unverified
                        let ip = parts[2].to_string();
                        if !beacons.iter().any(|b| b.ip == ip) {
                            beacons.push(BeaconDevice {
                                name: parts[0].to_string(),
                                signing_public_key: String::new(),
                                ssh_public_key: parts[1].to_string(),
                                ip,
                                port: 22,
                                timestamp: String::new(),
                                signature: String::new(),
                            });
                        }
                    }
                }
            }
            Err(_) => {} // read timeout: loop until the deadline
        }
    }
    beacons
}

/// Listens for beacons continuously and sends each via channel.
/// Runs indefinitely until channel is disconnected.
/// 
/// Only beacons with valid signatures are forwarded via the channel.
pub fn listen_beacons_continuous(port: u16, sender: std::sync::mpsc::Sender<BeaconDevice>) {
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
    
    // Maximum age for beacon messages (5 minutes = 300 seconds)
    let max_beacon_age = 300u64;
    
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let parts: Vec<&str> = text.split('\t').collect();
                
                // Expected format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
                if parts.len() >= 7 {
                    let ip = parts[3].to_string();
                    let _port: u16 = parts[4].parse().unwrap_or(22);
                    let timestamp: u64 = parts[5].parse().unwrap_or(0);
                    
                    // Check beacon age
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    
                    if now.saturating_sub(timestamp) > max_beacon_age {
                        eprintln!("WARNING: received expired beacon from {} (age: {}s)", 
                            ip, now.saturating_sub(timestamp));
                        continue;
                    }
                    
                    // Verify the signature
                    let beacon_str = parts.join("\t");
                    match signing::verify_beacon(&beacon_str, None) {
                        Ok((name, ssh_public_key, ip_verified, verified_port, timestamp_str)) => {
                            // Check IP consistency
                            if ip_verified != ip {
                                eprintln!("WARNING: beacon IP mismatch - claimed: {}, verified: {}", 
                                    ip, ip_verified);
                                continue;
                            }
                            
                            let device = BeaconDevice {
                                name,
                                signing_public_key: parts[1].to_string(),
                                ssh_public_key,
                                ip,
                                port: verified_port,
                                timestamp: timestamp_str,
                                signature: parts[6].to_string(),
                            };
                            let _ = sender.send(device);
                        }
                        Err(e) => {
                            eprintln!("WARNING: invalid beacon signature from {}: {}", ip, e);
                        }
                    }
                } else {
                    // Old format - backward compatibility
                    if parts.len() >= 3 {
                        eprintln!("WARNING: received beacon in old format from {}", parts[2]);
                        let device = BeaconDevice {
                            name: parts[0].to_string(),
                            signing_public_key: String::new(),
                            ssh_public_key: parts[1].to_string(),
                            ip: parts[2].to_string(),
                            port: 22,
                            timestamp: String::new(),
                            signature: String::new(),
                        };
                        let _ = sender.send(device);
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
