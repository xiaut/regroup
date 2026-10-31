//! SSH configuration module for regroup
//!
//! This module handles the generation and updating of SSH configuration files.

use std::fs;
use std::io;
use std::path::Path;

// ============================================================
// ~/.ssh/config
// ============================================================

/// Replaces ONLY the "Host <name>" block; all other blocks are preserved
/// (e.g. github.com). Idempotent: running it twice yields the same file.
pub fn update_block(
    path: &Path,
    name: &str,
    ip: &str,
    user: &str,
    port: u16,
    port_explicit: bool,
) -> io::Result<()> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut in_block = false;
    for line in old.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() && words[0] == "Host" {
            in_block = false;
            if words[1..].iter().any(|w| *w == name) {
                in_block = true;
                continue;
            }
        }
        if in_block {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let body = kept.trim_end();
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
    }
    out.push_str(&format!("Host {}\n    HostName {}\n", name, ip));
    if !user.is_empty() {
        out.push_str(&format!("    User {}\n", user));
    }
    if port_explicit {
        out.push_str(&format!("    Port {}\n", port));
    }
    fs::write(path, out)
}
