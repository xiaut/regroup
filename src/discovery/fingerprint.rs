//! SSH fingerprint module
//!
//! This module handles SSH key fingerprint collection and formatting.

use std::io::Write;
use std::process::{Command, Stdio};

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
