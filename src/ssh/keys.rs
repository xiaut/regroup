//! Type wrappers for SSH and signing keys
//!
//! This module provides newtype wrappers to prevent confusion between
//! different key formats and ensure type safety when passing keys around.
//!
//! # Usage
//!
//! Instead of passing raw strings for keys, use these wrapper types:
//!
//! ```rust
//! use crate::ssh::keys::{SshPublicKey, SigningPublicKeyB64};
//!
//! // Create a new SSH public key
//! let ssh_key = SshPublicKey::new("ssh-rsa AAAAB3NzaC1yc2E...".to_string());
//!
//! // Pass it to functions - type system ensures you can't accidentally
//! // pass a fingerprint where a public key is expected
//! some_function(&ssh_key);
//! ```
//!
//! This prevents bugs like the one where `get_ssh_server_fingerprint()` was used
//! instead of `get_ssh_server_public_key()`, causing devices to not find each other.

use std::fmt;

/// Wrapper for SSH server public key (full key, e.g., "ssh-rsa AAAAB3NzaC1yc2E..." or "ssh-ed25519 AAAAC3...")
/// 
/// This is the UNIQUE identifier for a machine. Machines are identified by their SSH public key,
/// not by hostname (which can change in Termux) or IP address (which can change with network).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SshPublicKey(pub String);

impl SshPublicKey {
    /// Create a new SSH public key wrapper
    pub fn new(key: String) -> Self {
        Self(key)
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
    
    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for SshPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SshPublicKey {
    fn from(key: String) -> Self {
        Self(key)
    }
}

impl From<&str> for SshPublicKey {
    fn from(key: &str) -> Self {
        Self(key.to_string())
    }
}

/// Wrapper for SSH fingerprint (hash of the public key, e.g., SHA256:... or MD5:...)
/// 
/// Fingerprints are used for display and verification, but NOT for machine identification.
/// Machine identification MUST use the full SSH public key, not the fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SshFingerprint(pub String);

impl SshFingerprint {
    /// Create a new SSH fingerprint wrapper
    pub fn new(fingerprint: String) -> Self {
        Self(fingerprint)
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SshFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Wrapper for ed25519 signing public key in base64 format
/// 
/// This is used for signing beacon messages. It's separate from the SSH public key
/// which is used for machine identification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SigningPublicKeyB64(pub String);

impl SigningPublicKeyB64 {
    /// Create a new signing public key wrapper
    pub fn new(key: String) -> Self {
        Self(key)
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SigningPublicKeyB64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Wrapper for ed25519 signature in base64 format
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureB64(pub String);

impl SignatureB64 {
    /// Create a new signature wrapper
    pub fn new(sig: String) -> Self {
        Self(sig)
    }
    
    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SignatureB64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
