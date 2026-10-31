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

/// Get all local IPv4 interfaces with their broadcast addresses
/// Returns a vector of (ip_address, broadcast_address) tuples
pub fn get_local_interfaces() -> Vec<(String, String)> {
    let mut interfaces = Vec::new();

    // Method 1: Try with `ip addr` (Linux)
    if let Ok(output) = Command::new("ip")
        .args(["-o", "addr", "show", "up"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let addr_part = parts[3];
                    if let Some(ip_str) = addr_part.split('/').next() {
                        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                            // Skip loopback
                            if ip.is_loopback() {
                                continue;
                            }
                            // Calculate broadcast address for the /24 subnet
                            let octets = ip.octets();
                            let broadcast = format!("{}.{}.{}.255", octets[0], octets[1], octets[2]);
                            // Only add if not already present
                            if !interfaces.iter().any(|(existing_ip, _)| *existing_ip == ip.to_string()) {
                                interfaces.push((ip.to_string(), broadcast));
                            }
                        }
                    }
                }
            }
        }
    }

    // Method 2: Fallback with `ifconfig` (Mac, BSD, etc.)
    if interfaces.is_empty() {
        if let Ok(output) = Command::new("ifconfig").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let trimmed = line.trim();
                    if trimmed.contains("inet ") && !trimmed.contains("127.0.0.1") && !trimmed.contains("::1") {
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        for part in &parts {
                            if let Ok(ip) = part.parse::<Ipv4Addr>() {
                                // Skip loopback
                                if ip.is_loopback() {
                                    continue;
                                }
                                let octets = ip.octets();
                                let broadcast = format!("{}.{}.{}.255", octets[0], octets[1], octets[2]);
                                if !interfaces.iter().any(|(existing_ip, _)| *existing_ip == ip.to_string()) {
                                    interfaces.push((ip.to_string(), broadcast));
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Method 3: Pure Rust fallback - try to discover interfaces
    if interfaces.is_empty() {
        // Try to get at least the default interface
        if let Some(ip) = my_ip() {
            if let Ok(ipv4) = ip.parse::<Ipv4Addr>() {
                let octets = ipv4.octets();
                let broadcast = format!("{}.{}.{}.255", octets[0], octets[1], octets[2]);
                interfaces.push((ip, broadcast));
            }
        }
    }

    interfaces
}
