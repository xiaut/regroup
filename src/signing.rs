//! Ed25519 signing module for beacon message authentication
//!
//! This module provides:
//! - Ed25519 keypair generation, loading, and saving
//! - Message signing and verification
//! - Key persistence in the root config directory (NOT in git)
//!
//! Security notes:
//! - The signing key pair is stored in binary format
//! - Private key (32 bytes seed) and public key (32 bytes) are stored in SEPARATE files
//! - The config/git/ directory contains only the public signing key (for shared config)
//! - The root config directory contains the private signing key (NOT in git)

use std::env;
use std::fs;
use std::path::PathBuf;

use ed25519_dalek::{Signer, Verifier, Signature, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;
use base64::Engine as _;

use crate::config::CONFIG_DIR;

// ============================================================
// Constants
// ============================================================

/// Directory for signing keys (in root config, NOT in git)
pub const SIGNING_KEY_DIR: &str = "signing";

/// Private key file name (32 bytes - the seed)
pub const PRIVATE_KEY_FILE: &str = "signing_private_key";

/// Public key file name (32 bytes)
pub const PUBLIC_KEY_FILE: &str = "signing_public_key";

// ============================================================
// Key persistence
// ============================================================

/// Get the signing keys directory path
fn get_signing_key_dir() -> PathBuf {
    let home = env::var("HOME").expect("HOME variable not set");
    PathBuf::from(home).join(CONFIG_DIR).join(SIGNING_KEY_DIR)
}

/// Get the private key file path
pub fn get_private_key_path() -> PathBuf {
    get_signing_key_dir().join(PRIVATE_KEY_FILE)
}

/// Get the public key file path
pub fn get_public_key_path() -> PathBuf {
    get_signing_key_dir().join(PUBLIC_KEY_FILE)
}

/// Generate a new Ed25519 keypair
/// 
/// Returns (signing_key, verifying_key) tuple
pub fn generate_keypair() -> Result<(SigningKey, VerifyingKey), String> {
    let mut csprng = OsRng;
    
    // Generate a random secret key (32 bytes)
    let mut secret_key = [0u8; 32];
    csprng.fill_bytes(&mut secret_key);
    
    // Create signing key from secret key
    let signing_key = SigningKey::from_bytes(&secret_key);
    let verifying_key = signing_key.verifying_key();
    Ok((signing_key, verifying_key))
}

/// Save the private signing key to disk (32 bytes, binary)
/// 
/// SECURITY: This file contains sensitive data and should NEVER be committed to git
/// The file is stored in ~/.regroup-config/signing/signing_private_key
pub fn save_private_key(signing_key: &SigningKey) -> Result<(), String> {
    let key_dir = get_signing_key_dir();
    let key_path = get_private_key_path();
    
    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(&key_dir) {
        return Err(format!("cannot create signing key directory: {}", e));
    }
    
    // Save private key as binary (32 bytes - the seed)
    let private_bytes = signing_key.to_bytes();
    if let Err(e) = fs::write(&key_path, private_bytes) {
        return Err(format!("cannot write private key: {}", e));
    }
    
    // Set restrictive permissions (read/write only by owner)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)) {
            return Err(format!("cannot set permissions on private key: {}", e));
        }
    }
    
    Ok(())
}

/// Save the public signing key to disk (32 bytes, binary)
/// 
/// This can be safely shared and committed to git as it's public data
pub fn save_public_key(verifying_key: &VerifyingKey) -> Result<(), String> {
    let key_dir = get_signing_key_dir();
    let key_path = get_public_key_path();
    
    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(&key_dir) {
        return Err(format!("cannot create signing key directory: {}", e));
    }
    
    // Save public key as binary (32 bytes)
    let public_bytes = verifying_key.to_bytes();
    if let Err(e) = fs::write(&key_path, public_bytes) {
        return Err(format!("cannot write public key: {}", e));
    }
    
    Ok(())
}

/// Load the private signing key from disk
/// 
/// Returns None if the key doesn't exist
pub fn load_private_key() -> Option<SigningKey> {
    let key_path = get_private_key_path();
    
    if !key_path.exists() {
        return None;
    }
    
    let private_bytes = match fs::read(&key_path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    
    // Private key seed is 32 bytes
    if private_bytes.len() != 32 {
        eprintln!("WARNING: private key has unexpected length: {}", private_bytes.len());
        return None;
    }
    
    // Try to parse the seed
    let seed = Zeroizing::new(private_bytes.try_into().ok()?);
    // SigningKey::from_bytes takes a SecretKey (&[u8; 32])
    // In ed25519-dalek, the seed IS the secret key
    let result = SigningKey::from_bytes(&seed);
    Some(result)
}

/// Load the public signing key from disk
/// 
/// Returns None if the key doesn't exist
pub fn load_public_key() -> Option<VerifyingKey> {
    let key_path = get_public_key_path();
    
    if !key_path.exists() {
        return None;
    }
    
    let public_bytes = match fs::read(&key_path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    
    // Public key is 32 bytes
    if public_bytes.len() != 32 {
        eprintln!("WARNING: public key has unexpected length: {}", public_bytes.len());
        return None;
    }
    
    match VerifyingKey::from_bytes(&public_bytes.try_into().unwrap()) {
        Ok(key) => Some(key),
        Err(_) => None,
    }
}

// ============================================================
// Signing and verification
// ============================================================

/// Sign a message with the given signing key
/// 
/// The message format for beacon signing is: "name|ssh_public_key|ip|port|timestamp"
pub fn sign_message(message: &str, signing_key: &SigningKey) -> Result<String, String> {
    let signature = signing_key.sign(message.as_bytes());
    
    // Encode signature as base64 for transmission
    let signature_bytes = signature.to_bytes();
    let base64_engine = base64::engine::general_purpose::STANDARD;
    Ok(base64_engine.encode(&signature_bytes))
}

/// Verify a message signature with the given public key
/// 
/// Returns true if the signature is valid
pub fn verify_signature(
    message: &str,
    signature_b64: &str,
    verifying_key: &VerifyingKey,
) -> Result<bool, String> {
    let base64_engine = base64::engine::general_purpose::STANDARD;
    
    let signature_bytes = match base64_engine.decode(signature_b64) {
        Ok(b) => b,
        Err(e) => return Err(format!("invalid base64 signature: {}", e)),
    };
    
    // Signature is 64 bytes
    if signature_bytes.len() != 64 {
        return Err(format!("invalid signature length: {}", signature_bytes.len()));
    }
    
    // Signature::from_bytes takes &[u8; 64] and returns Result<Signature, SignatureError>
    // We need to convert Vec<u8> to [u8; 64]
    let signature = Signature::from_bytes(&signature_bytes.try_into().map_err(
        |_| "invalid signature length"
    )?);
    
    // Note: In ed25519-dalek 2.x, from_bytes may still fail for invalid signatures
    // but the error type doesn't implement Display, so we'll just use the signature
    
    Ok(verifying_key.verify(message.as_bytes(), &signature).is_ok())
}

/// Sign a beacon message
/// 
/// Creates the signed beacon string in the format:
/// "name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64"
pub fn sign_beacon(
    name: &str,
    ssh_public_key: &str,
    ip: &str,
    port: u16,
    timestamp: &str,
    signing_key: &SigningKey,
) -> Result<String, String> {
    // Create the message to sign: "name|ssh_public_key|ip|port|timestamp"
    let message = format!("{}|{}|{}|{}|{}", name, ssh_public_key, ip, port, timestamp);
    
    // Sign the message
    let signature_b64 = sign_message(&message, signing_key)?;
    
    // Get the verifying key from the signing key
    let verifying_key = signing_key.verifying_key();
    
    // Encode the signing public key as base64
    let base64_engine = base64::engine::general_purpose::STANDARD;
    let signing_public_key_b64 = base64_engine.encode(&verifying_key.to_bytes());
    
    // Create the beacon string with tab-separated fields
    // Format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
    Ok(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        name,
        signing_public_key_b64,
        ssh_public_key,
        ip,
        port,
        timestamp,
        signature_b64
    ))
}

/// Verify a beacon signature
/// 
/// Parses a beacon string and verifies its signature
/// Returns (name, ssh_public_key, ip, port, timestamp) if valid, or an error
pub fn verify_beacon(
    beacon_str: &str,
    expected_signing_public_key: Option<&VerifyingKey>,
) -> Result<(String, String, String, u16, String), String> {
    let parts: Vec<&str> = beacon_str.split('\t').collect();
    
    // Expected format: name\tsigning_public_key_b64\tssh_public_key\tip\tport\ttimestamp\tsignature_b64
    // That's 7 fields total
    if parts.len() != 7 {
        return Err(format!("invalid beacon format: expected 7 fields, got {}", parts.len()));
    }
    
    let name = parts[0].to_string();
    let signing_public_key_b64 = parts[1];
    let ssh_public_key = parts[2].to_string();
    let ip = parts[3].to_string();
    let port: u16 = parts[4].parse().map_err(|_| "invalid port number".to_string())?;
    let timestamp = parts[5].to_string();
    let signature_b64 = parts[6];
    
    // Decode the signing public key
    let base64_engine = base64::engine::general_purpose::STANDARD;
    
    let signing_public_key_bytes = match base64_engine.decode(signing_public_key_b64) {
        Ok(b) => b,
        Err(e) => return Err(format!("invalid base64 signing public key: {}", e)),
    };
    
    let signing_public_key = match VerifyingKey::from_bytes(&signing_public_key_bytes.try_into().unwrap()) {
        Ok(key) => key,
        Err(e) => return Err(format!("invalid signing public key bytes: {}", e)),
    };
    
    // If we have an expected signing public key, verify it matches
    if let Some(expected_key) = expected_signing_public_key {
        if signing_public_key.to_bytes() != expected_key.to_bytes() {
            return Err("signing public key mismatch".to_string());
        }
    }
    
    // Reconstruct the message to verify
    let message = format!("{}|{}|{}|{}|{}", name, ssh_public_key, ip, port, timestamp);
    
    // Verify the signature
    let is_valid = verify_signature(&message, signature_b64, &signing_public_key)?;
    
    if is_valid {
        Ok((name, ssh_public_key, ip, port, timestamp))
    } else {
        Err("invalid signature".to_string())
    }
}

// ============================================================
// Utility functions
// ============================================================

/// Get the signing public key in base64 format for display
pub fn get_signing_public_key_b64() -> Option<String> {
    let base64_engine = base64::engine::general_purpose::STANDARD;
    load_public_key().map(|key| base64_engine.encode(&key.to_bytes()))
}

/// Check if signing keys exist
pub fn signing_keys_exist() -> bool {
    get_private_key_path().exists() && get_public_key_path().exists()
}

/// Initialize signing keys (generate if they don't exist)
/// 
/// Returns (signing_key, verifying_key) tuple
pub fn initialize_signing_keys() -> Result<(SigningKey, VerifyingKey), String> {
    if signing_keys_exist() {
        // Try to load existing keys
        if let (Some(signing_key), Some(verifying_key)) = (load_private_key(), load_public_key()) {
            // Verify they match
            if signing_key.verifying_key().to_bytes() == verifying_key.to_bytes() {
                return Ok((signing_key, verifying_key));
            }
            // Keys don't match - regenerate
            eprintln!("WARNING: signing key pair inconsistent, regenerating...");
        }
    }
    
    // Generate new keypair
    let (signing_key, verifying_key) = generate_keypair()?;
    save_private_key(&signing_key)?;
    save_public_key(&verifying_key)?;
    
    Ok((signing_key, verifying_key))
}
