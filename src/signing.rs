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

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::config::CONFIG_DIR;
use crate::ssh::keys::{SignatureB64, SshPublicKey};

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
fn get_signing_key_dir() -> Result<PathBuf, String> {
    let home = env::var("HOME").map_err(|_| "HOME variable not set".to_string())?;
    Ok(PathBuf::from(home).join(CONFIG_DIR).join(SIGNING_KEY_DIR))
}

/// Get the private key file path
pub fn get_private_key_path() -> Result<PathBuf, String> {
    Ok(get_signing_key_dir()?.join(PRIVATE_KEY_FILE))
}

/// Get the public key file path
pub fn get_public_key_path() -> Result<PathBuf, String> {
    Ok(get_signing_key_dir()?.join(PUBLIC_KEY_FILE))
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
    let key_dir = get_signing_key_dir()?;
    let key_path = get_private_key_path()?;

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
    let key_dir = get_signing_key_dir()?;
    let key_path = get_public_key_path()?;

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
    let key_path = get_private_key_path().ok()?;

    if !key_path.exists() {
        return None;
    }

    let private_bytes = match fs::read(&key_path) {
        Ok(b) => b,
        Err(_) => return None,
    };

    // Private key seed is 32 bytes
    if private_bytes.len() != 32 {
        eprintln!(
            "WARNING: private key has unexpected length: {}",
            private_bytes.len()
        );
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
    let key_path = get_public_key_path().ok()?;

    if !key_path.exists() {
        return None;
    }

    let public_bytes = match fs::read(&key_path) {
        Ok(b) => b,
        Err(_) => return None,
    };

    // Public key is 32 bytes
    if public_bytes.len() != 32 {
        eprintln!(
            "WARNING: public key has unexpected length: {}",
            public_bytes.len()
        );
        return None;
    }

    // Length is checked above; try_into cannot fail here, but do not unwrap
    let public_array: [u8; 32] = public_bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&public_array).ok()
}

// ============================================================
// Signing and verification
// ============================================================

/// Sign a message with the given signing key
///
/// The message format for beacon signing is: "name|ssh_public_key|ip|port|timestamp"
pub fn sign_message(message: &str, signing_key: &SigningKey) -> Result<SignatureB64, String> {
    let signature = signing_key.sign(message.as_bytes());

    // Encode signature as base64 for transmission
    let signature_bytes = signature.to_bytes();
    let base64_engine = base64::engine::general_purpose::STANDARD;
    Ok(SignatureB64::new(base64_engine.encode(signature_bytes)))
}

/// Verify a message signature with the given public key
///
/// Returns true if the signature is valid
pub fn verify_signature(
    message: &str,
    signature_b64: &SignatureB64,
    verifying_key: &VerifyingKey,
) -> Result<bool, String> {
    let base64_engine = base64::engine::general_purpose::STANDARD;

    let signature_bytes = match base64_engine.decode(signature_b64.as_str()) {
        Ok(b) => b,
        Err(e) => return Err(format!("invalid base64 signature: {}", e)),
    };

    // Signature is 64 bytes
    if signature_bytes.len() != 64 {
        return Err(format!(
            "invalid signature length: {}",
            signature_bytes.len()
        ));
    }

    // Signature::from_bytes takes &[u8; 64] and returns Result<Signature, SignatureError>
    // We need to convert Vec<u8> to [u8; 64]
    let signature = Signature::from_bytes(
        &signature_bytes
            .try_into()
            .map_err(|_| "invalid signature length")?,
    );

    // Note: In ed25519-dalek 2.x, from_bytes may still fail for invalid signatures
    // but the error type doesn't implement Display, so we'll just use the signature

    Ok(verifying_key.verify(message.as_bytes(), &signature).is_ok())
}

/// Sign a beacon message
///
/// Creates the signed beacon string in the format:
/// "name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64"
pub fn sign_beacon(
    name: &str,
    ssh_public_key: &SshPublicKey,
    user: &str,
    ip: &str,
    port: u16,
    timestamp: &str,
    signing_key: &SigningKey,
) -> Result<String, String> {
    // Create the message to sign: "name|ssh_public_key|user|ip|port|timestamp"
    let message = format!("{}|{}|{}|{}|{}|{}", name, ssh_public_key, user, ip, port, timestamp);

    // Sign the message
    let signature_b64 = sign_message(&message, signing_key)?;

    // Get the verifying key from the signing key
    let verifying_key = signing_key.verifying_key();

    // Encode the signing public key as base64
    let base64_engine = base64::engine::general_purpose::STANDARD;
    let signing_public_key_b64 = base64_engine.encode(verifying_key.to_bytes());

    // Create the beacon string with tab-separated fields
    // Format: name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64
    Ok(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        name, signing_public_key_b64, ssh_public_key, user, ip, port, timestamp, signature_b64
    ))
}

/// Verify a beacon signature
///
/// Parses a beacon string and verifies its signature
/// Returns (name, ssh_public_key, user, ip, port) if valid, or an error
pub fn verify_beacon(
    beacon_str: &str,
    expected_signing_public_key: Option<&VerifyingKey>,
) -> Result<(String, SshPublicKey, String, String, u16), String> {
    let parts: Vec<&str> = beacon_str.split('\t').collect();

    // Expected format: name\tsigning_public_key_b64\tssh_public_key\tuser\tip\tport\ttimestamp\tsignature_b64
    // That's 8 fields total
    if parts.len() != 8 {
        return Err(format!(
            "invalid beacon format: expected 8 fields, got {}",
            parts.len()
        ));
    }

    let name = parts[0].to_string();
    let signing_public_key_b64 = parts[1];
    let ssh_public_key = SshPublicKey::from(parts[2]);
    let user = parts[3].to_string();
    let ip = parts[4].to_string();
    let port: u16 = parts[5]
        .parse()
        .map_err(|_| "invalid port number".to_string())?;
    let timestamp = parts[6].to_string();
    let signature_b64 = SignatureB64::from(parts[7]);

    // Decode the signing public key
    let base64_engine = base64::engine::general_purpose::STANDARD;

    let signing_public_key_bytes = match base64_engine.decode(signing_public_key_b64) {
        Ok(b) => b,
        Err(e) => return Err(format!("invalid base64 signing public key: {}", e)),
    };

    // The signing public key is attacker-controlled network input: validate
    // its length explicitly and never unwrap (a beacon with a key of the
    // wrong length used to panic the whole process).
    let signing_public_key_len = signing_public_key_bytes.len();
    let signing_public_key_array: [u8; 32] = match signing_public_key_bytes.try_into() {
        Ok(a) => a,
        Err(_) => {
            return Err(format!(
                "invalid signing public key length: {} bytes (expected 32)",
                signing_public_key_len
            ));
        }
    };

    let signing_public_key = match VerifyingKey::from_bytes(&signing_public_key_array) {
        Ok(key) => key,
        Err(e) => return Err(format!("invalid signing public key bytes: {}", e)),
    };

    // If we have an expected signing public key, verify it matches
    if let Some(expected_key) = expected_signing_public_key
        && signing_public_key.to_bytes() != expected_key.to_bytes()
    {
        return Err("signing public key mismatch".to_string());
    }

    // Reconstruct the message to verify
    let message = format!("{}|{}|{}|{}|{}|{}", name, ssh_public_key, user, ip, port, timestamp);

    // Verify the signature
    let is_valid = verify_signature(&message, &signature_b64, &signing_public_key)?;

    if is_valid {
        Ok((name, ssh_public_key, user, ip, port))
    } else {
        Err("invalid signature".to_string())
    }
}

// ============================================================
// Utility functions
// ============================================================

/// Check if signing keys exist
pub fn signing_keys_exist() -> bool {
    get_private_key_path().is_ok_and(|p| p.exists())
        && get_public_key_path().is_ok_and(|p| p.exists())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_key() -> SshPublicKey {
        SshPublicKey::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFake")
    }

    fn signed_beacon() -> (String, SigningKey) {
        let (signing_key, _) = generate_keypair().expect("keypair generation");
        let beacon = sign_beacon("selftest", &ssh_key(), "", "10.1.2.3", 22, "0", &signing_key)
            .expect("beacon signing");
        (beacon, signing_key)
    }

    #[test]
    fn sign_beacon_verify_roundtrip() {
        let (beacon, signing_key) = signed_beacon();
        let verifying_key = signing_key.verifying_key();

        let (name, key, user, ip, port) =
            verify_beacon(&beacon, Some(&verifying_key)).expect("a beacon we signed must verify");
        assert_eq!(name, "selftest");
        assert_eq!(key.as_str(), ssh_key().as_str());
        assert_eq!(user, "");
        assert_eq!(ip, "10.1.2.3");
        assert_eq!(port, 22);
    }

    #[test]
    fn tampered_beacon_field_breaks_signature() {
        let (beacon, signing_key) = signed_beacon();
        let verifying_key = signing_key.verifying_key();

        let tampered = beacon.replace("10.1.2.3", "10.9.9.9");
        assert!(verify_beacon(&tampered, Some(&verifying_key)).is_err());

        let truncated = beacon.replace("selftest", "attacker");
        assert!(verify_beacon(&truncated, Some(&verifying_key)).is_err());
    }

    #[test]
    fn beacon_signed_with_wrong_key_is_rejected() {
        let (beacon, _) = signed_beacon();
        let (other_key, _) = generate_keypair().expect("keypair generation");
        let other_verifying = other_key.verifying_key();

        // A pinned key that did not sign this beacon must fail the check.
        assert!(verify_beacon(&beacon, Some(&other_verifying)).is_err());
    }

    #[test]
    fn malformed_beacon_is_rejected_without_panicking() {
        // Not enough fields
        assert!(verify_beacon("only\tthree\tfields", None).is_err());
        // Malformed signing key (base64 that decodes to 3 bytes, not 32)
        let malformed = "selftest\tAAAA\tssh-ed25519 AAAA\tuser\t10.1.2.3\t22\t0\tAAAA";
        assert!(verify_beacon(malformed, None).is_err());
        // Non-numeric port
        let (beacon, _) = signed_beacon();
        let parts: Vec<&str> = beacon.split('\t').collect();
        let bad_port = format!(
            "{}\t{}\t{}\t{}\t{}\txx\t{}\t{}",
            parts[0], parts[1], parts[2], parts[3], parts[4], parts[6], parts[7]
        );
        assert!(verify_beacon(&bad_port, None).is_err());
    }
}
