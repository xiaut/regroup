//! Utility functions for interactive mode.

use crate::ssh::keys::SshPublicKey;

/// Short preview of an SSH public key for display, e.g. "ssh-ed25519 AA... ".
pub fn key_preview(key: &SshPublicKey) -> String {
    let s = key.as_str();
    if s.len() > 16 {
        format!("{}...", &s[..13])
    } else {
        s.to_string()
    }
}
