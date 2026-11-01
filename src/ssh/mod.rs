//! SSH module for regroup
//!
//! This module handles SSH-related functionality.

pub mod config;
pub mod config_helpers;
pub mod keys;
pub mod public_key;

pub use config::update_block;
pub use config_helpers::{flush_ssh_config, verify_ssh_config_complete};
// Type wrappers for SSH keys (available for use to prevent type confusion)
pub use keys::{SshPublicKey, SshFingerprint, SigningPublicKeyB64, SignatureB64};
