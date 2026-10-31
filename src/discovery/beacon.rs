//! UDP beacon module
//!
//! This module handles UDP beacon announcement and listening for device discovery.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Broadcasts "name ip" every 2 s until the deadline on all local interfaces.
/// Runs in its own thread, in parallel with listen_beacons, for the same window.
/// Each interface sends to its own broadcast address.
pub fn announce_beacon(port: u16, secs: u64, name: &str, interfaces: &[(String, String)]) {
    if interfaces.is_empty() {
        eprintln!("WARNING: no local interfaces found — announcing disabled");
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(secs);
    let message = format!("{} {}", name, &interfaces[0].0);

    // Create one socket per interface to send on the correct broadcast address
    let mut sockets = Vec::new();
    for (ip, broadcast) in interfaces {
        if let Ok(sock) = std::net::UdpSocket::bind(format!("{}:0", ip)) {
            if sock.set_broadcast(true).is_ok() {
                sockets.push((sock, broadcast.clone()));
            }
        }
    }

    if sockets.is_empty() {
        eprintln!("WARNING: cannot bind to any interface for announcing");
        return;
    }

    while Instant::now() < deadline {
        for (sock, broadcast) in &sockets {
            let target = format!("{}:{}", broadcast, port);
            if let Ok(addr) = target.parse::<SocketAddr>() {
                let _ = sock.send_to(message.as_bytes(), addr);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Listens for "name ip" beacons during the window. Returns (name, ip) pairs,
/// deduplicated by IP.
pub fn listen_beacons(port: u16, secs: u64) -> Vec<(String, String)> {
    let mut beacons: Vec<(String, String)> = Vec::new();
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
                let words: Vec<&str> = text.split_whitespace().collect();
                for w in &words {
                    if let Ok(ip) = w.parse::<Ipv4Addr>() {
                        let s = ip.to_string();
                        if !beacons.iter().any(|(_, i)| i == &s) {
                            let name = words
                                .iter()
                                .find(|x| x.parse::<Ipv4Addr>().is_err())
                                .map(|x| x.to_string())
                                .unwrap_or_default();
                            beacons.push((name, s));
                        }
                        break;
                    }
                }
            }
            Err(_) => {} // read timeout: loop until the deadline
        }
    }
    beacons
}
