//! Discovery module for regroup
//!
//! This module handles all network discovery functionality including:
//! - UDP beacon announcement and listening
//! - TCP subnet scanning
//! - SSH fingerprint detection
//! - Local identity detection
//! - Discovery and sync loop

pub mod sync;

use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ============================================================
// Local identity
// ============================================================

/// Get the local hostname
pub fn host_name() -> String {
    if let Ok(o) = Command::new("hostname").output() {
        if o.status.success() {
            let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !n.is_empty() {
                return n;
            }
        }
    }
    "machine".to_string()
}

/// Get the local IP address
pub fn my_ip() -> Option<String> {
    // "Connecting" a UDP socket sends no packet but selects the default
    // interface and reveals its source address.
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("1.1.1.1:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                let ip = a.ip().to_string();
                if ip.parse::<Ipv4Addr>().is_ok() {
                    return Some(ip);
                }
            }
        }
    }
    // Fallback: ask the kernel via "ip route get".
    if let Ok(o) = Command::new("ip")
        .args(["route", "get", "1.1.1.1"])
        .output()
    {
        if o.status.success() {
            let words: Vec<String> = String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(ToString::to_string)
                .collect();
            for (i, w) in words.iter().enumerate() {
                if *w == "src" && i + 1 < words.len() {
                    return Some(words[i + 1].to_string());
                }
            }
        }
    }
    None
}

/// Get the fingerprint of the local machine
pub fn get_local_fingerprint(port: u16) -> Option<String> {
    if let Some(ip) = my_ip() {
        let fps = fingerprints(&ip, port, false);
        if !fps.is_empty() {
            return Some(fps[0].clone());
        }
    }
    None
}

// ============================================================
// Subnet helpers
// ============================================================

/// Keeps at most the first three dot-separated parts ("192.168.43.0/24" -> "192.168.43").
pub fn net_prefix(s: &str) -> String {
    let parts: Vec<&str> = s.split('.').collect();
    let n = parts.len().min(3);
    parts[..n].join(".")
}

/// Normalizes a /24 prefix; Some("192.168.43") if valid, None otherwise.
pub fn net_valid(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut octets: Vec<u32> = Vec::new();
    for p in parts {
        if p.is_empty() || p.len() > 3 {
            return None;
        }
        if !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: u32 = match p.parse() {
            Ok(v) => v,
            Err(_) => return None,
        };
        if v > 255 {
            return None;
        }
        octets.push(v);
    }
    let out: Vec<String> = octets.iter().map(|o| o.to_string()).collect();
    Some(out.join("."))
}

// ============================================================
// Beacons (UDP)
// ============================================================

/// Broadcasts "name ip" every 2 s until the deadline. Runs in its own thread,
/// in parallel with listen_beacons, for the same window.
pub fn announce_beacon(port: u16, secs: u64, name: &str, ip: &str) {
    let target = format!("255.255.255.255:{}", port);
    let message = format!("{} {}", name, ip);
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING: announcing disabled: {}", e);
            return;
        }
    };
    let _ = sock.set_broadcast(true);
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        let _ = sock.send_to(message.as_bytes(), &target);
        thread::sleep(Duration::from_secs(2));
    }
}

/// Listens for "name ip" beacons during the window. Returns (name, ip) pairs,
/// deduplicated by IP.
pub fn listen_beacons(port: u16, secs: u64) -> Vec<(String, String)> {
    let mut beacons: Vec<(String, String)> = Vec::new();
    let sock = match UdpSocket::bind(("0.0.0.0", port)) {
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

// ============================================================
// Fallback TCP scan
// ============================================================

/// Scans a /24 on one port, `batch_size` connections in parallel.
pub fn scan_subnet(net: &str, port: u16, batch_size: usize) -> Vec<(String, u16)> {
    let results = Arc::new(Mutex::new(Vec::<(String, u16)>::new()));
    let mut start: u32 = 1;
    while start <= 254 {
        let end = (start + batch_size as u32 - 1).min(254);
        let mut handles = Vec::new();
        for k in start..=end {
            let net2 = net.to_string();
            let results2 = Arc::clone(&results);
            handles.push(thread::spawn(move || {
                let addr = format!("{}.{}:{}", net2, k, port);
                if let Ok(sa) = addr.parse::<SocketAddr>() {
                    if TcpStream::connect_timeout(&sa, Duration::from_secs(1)).is_ok() {
                        results2
                            .lock()
                            .unwrap()
                            .push((format!("{}.{}", net2, k), port));
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        start = end + 1;
    }
    results.lock().unwrap().clone()
}

// ============================================================
// Fingerprints (ssh-keyscan | ssh-keygen)
// ============================================================

/// Collects the fingerprints of every host key offered by ip:port, using
/// OpenSSH itself: ssh-keyscan fetches the keys, ssh-keygen -lf - prints
/// "bits SHA256:... comment (type)" lines.
pub fn fingerprints(ip: &str, port: u16, md5: bool) -> Vec<String> {
    let scan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("-p")
        .arg(port.to_string())
        .arg(ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let raw = match scan {
        Ok(o) if !o.stdout.is_empty() => o.stdout,
        _ => return Vec::new(),
    };
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if md5 {
        cmd.arg("-E").arg("md5");
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&raw);
    } // stdin dropped here -> EOF for ssh-keygen
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let wanted = if md5 { "MD5:" } else { "SHA256:" };
    let mut fps: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 && fields[1].starts_with(wanted) {
            let f = fields[1].to_string();
            if !fps.contains(&f) {
                fps.push(f);
            }
        }
    }
    fps
}
