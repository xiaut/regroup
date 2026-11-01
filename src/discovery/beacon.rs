//! UDP beacon module
//!
//! This module handles UDP beacon announcement and listening for device discovery.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::NetworkInterface;

/// Represents a device announced via UDP beacon
#[derive(Clone, Debug)]
pub struct BeaconDevice {
    pub name: String,
    pub public_key: String,
    pub ip: String,
}

/// Broadcasts "name\tpublic_key\tip" every 2 s until the deadline on all local interfaces.
/// Runs in its own thread, in parallel with listen_beacons, for the same window.
/// Each interface sends to its own broadcast address.
pub fn announce_beacon(port: u16, secs: u64, name: &str, public_key: &str, interfaces: &[NetworkInterface]) {
    if interfaces.is_empty() {
        eprintln!("WARNING: no local interfaces found — announcing disabled");
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(secs);

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

    while Instant::now() < deadline {
        for (sock, broadcast, ip) in &sockets {
            let message = format!("{}\t{}\t{}", name, public_key, ip);
            let target = format!("{}:{}", broadcast, port);
            if let Ok(addr) = target.parse::<SocketAddr>() {
                let _ = sock.send_to(message.as_bytes(), addr);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Listens for "name\tpublic_key\tip" beacons during the window. Returns BeaconDevice list,
/// deduplicated by IP.
pub fn listen_beacons(port: u16, secs: u64) -> Vec<BeaconDevice> {
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
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let parts: Vec<&str> = text.split('\t').collect();
                if parts.len() >= 3 {
                    let ip = parts[2].to_string();
                    if !beacons.iter().any(|b| b.ip == ip) {
                        beacons.push(BeaconDevice {
                            name: parts[0].to_string(),
                            public_key: parts[1].to_string(),
                            ip,
                        });
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
pub fn listen_beacons_continuous(port: u16, sender: std::sync::mpsc::Sender<BeaconDevice>) {
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
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let parts: Vec<&str> = text.split('\t').collect();
                if parts.len() >= 3 {
                    let device = BeaconDevice {
                        name: parts[0].to_string(),
                        public_key: parts[1].to_string(),
                        ip: parts[2].to_string(),
                    };
                    let _ = sender.send(device);
                }
            }
            Err(_) => {
                // Read timeout, sleep briefly to avoid busy loop
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
