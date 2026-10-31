//! Network helpers module
//!
//! This module handles subnet prefix operations.

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
