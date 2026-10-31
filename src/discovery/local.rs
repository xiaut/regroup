//! Local identity module
//!
//! This module handles local machine identification.

use std::net::Ipv4Addr;
use std::process::Command;

use crate::discovery::fingerprint::fingerprints;

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
    use std::net::UdpSocket;
    
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
