//! Device configuration module
//!
//! This module handles loading and saving individual device configurations.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use super::{DeviceName, Machine};
use super::json_helpers::int_value;
use super::validation::validate_machine;
use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};

/// Load a single device configuration from a JSON file
pub fn load_device_file(path: &Path) -> Result<Machine, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Err(format!("cannot read: {}", e)),
    };

    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON: {}", e)),
    };

    let obj = match root.as_object() {
        Some(o) => o,
        None => return Err("top-level value must be a JSON object".to_string()),
    };

    let name = match obj.get("name") {
        Some(v) => match v.as_str() {
            Some(s) => {
                DeviceName::from_str(s)
                    .ok_or_else(|| "\"name\" must be a non-empty string".to_string())?
            }
            None => return Err("\"name\" must be a string".to_string()),
        },
        None => {
            // Try to get name from filename
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let name_str = if let Some(name) = filename.strip_suffix(".json") {
                name.to_string()
            } else {
                filename.to_string()
            };
            DeviceName::new(name_str)
                .ok_or_else(|| "Cannot determine device name from filename".to_string())?
        }
    };

    let ssh_public_key = match obj.get("ssh_public_key") {
        Some(v) => match v.as_str() {
            Some(s) => SshPublicKey::from(s.trim()),
            None => return Err("\"ssh_public_key\" must be a string".to_string()),
        },
        None => return Err("\"ssh_public_key\" is required".to_string()),
    };

    let user = match obj.get("user") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err("\"user\" must be a string".to_string()),
        },
        None => String::new(),
    };

    let signing_public_key = match obj.get("signing_public_key") {
        Some(v) => match v.as_str() {
            Some(s) => SigningPublicKeyB64::from(s.trim()),
            None => return Err("\"signing_public_key\" must be a string".to_string()),
        },
        None => return Err("\"signing_public_key\" is required".to_string()),
    };

    let (port, port_explicit) = match obj.get("port") {
        None => (22u16, false),
        Some(v) => {
            let n = match int_value(v) {
                Some(n) => n,
                None => return Err("\"port\" must be an integer".to_string()),
            };
            if !(1..=65535).contains(&n) {
                return Err("\"port\" must be between 1 and 65535".to_string());
            }
            (n as u16, true)
        }
    };

    let machine = Machine {
        name,
        ssh_public_key,
        signing_public_key,
        user,
        port,
        port_explicit,
    };

    // Validate the machine configuration
    validate_machine(&machine)?;

    Ok(machine)
}

/// Save a device configuration to a JSON file
pub fn save_device_file(path: &Path, machine: &Machine) -> io::Result<()> {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "name".to_string(),
        serde_json::Value::String(machine.name.as_str().to_string()),
    );
    obj.insert(
        "ssh_public_key".to_string(),
        serde_json::Value::String(machine.ssh_public_key.0.clone()),
    );
    obj.insert(
        "signing_public_key".to_string(),
        serde_json::Value::String(machine.signing_public_key.0.clone()),
    );

    if !machine.user.is_empty() {
        obj.insert(
            "user".to_string(),
            serde_json::Value::String(machine.user.clone()),
        );
    }

    if machine.port_explicit {
        obj.insert(
            "port".to_string(),
            serde_json::Value::Number(machine.port.into()),
        );
    }

    let value = serde_json::Value::Object(obj);
    fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique temp directory for a test, cleaned up on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "regroup-device-test-{}-{}",
                tag,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn path(&self, file: &str) -> std::path::PathBuf {
            self.0.join(file)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const VALID_DEVICE: &str = r#"{
        "name": "laptop",
        "ssh_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForTests",
        "signing_public_key": "MCowBQYDK2VwAyEAGXNlbGZ0ZXN0a2V5eXl5eXl5eXl5eXl5eXl5eXl5eXk=",
        "user": "alice",
        "port": 8022
    }"#;

    #[test]
    fn load_device_file_parses_all_fields() {
        let dir = TempDir::new("parse");
        let path = dir.path("laptop.json");
        fs::write(&path, VALID_DEVICE).unwrap();

        let m = load_device_file(&path).expect("valid device file must load");
        assert_eq!(m.name.as_str(), "laptop");
        assert_eq!(m.user, "alice");
        assert_eq!(m.port, 8022);
        assert!(m.port_explicit);
        assert_eq!(
            m.ssh_public_key.as_str(),
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForTests"
        );
        assert_eq!(
            m.signing_public_key.as_str(),
            "MCowBQYDK2VwAyEAGXNlbGZ0ZXN0a2V5eXl5eXl5eXl5eXl5eXl5eXl5eXk="
        );
    }

    #[test]
    fn load_device_file_requires_ssh_public_key() {
        let dir = TempDir::new("no-ssh-key");
        let path = dir.path("a.json");
        fs::write(
            &path,
            r#"{"name": "a", "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
        )
        .unwrap();

        let err = load_device_file(&path).unwrap_err();
        assert!(err.contains("ssh_public_key"), "unexpected error: {err}");
    }

    #[test]
    fn load_device_file_requires_signing_public_key() {
        let dir = TempDir::new("no-signing-key");
        let path = dir.path("b.json");
        fs::write(&path, r#"{"name": "b", "ssh_public_key": "ssh-rsa AAAA"}"#).unwrap();

        let err = load_device_file(&path).unwrap_err();
        assert!(
            err.contains("signing_public_key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_device_file_rejects_out_of_range_port() {
        let dir = TempDir::new("bad-port");
        for port in ["0", "65536", "not-a-number"] {
            let path = dir.path(format!("p{port}.json").as_str());
            fs::write(
                &path,
                format!(
                    r#"{{"name": "p", "ssh_public_key": "ssh-rsa AAAA",
                    "signing_public_key": "MCowBQYDK2VwAyEA==", "port": {port}}}"#
                ),
            )
            .unwrap();
            assert!(
                load_device_file(&path).is_err(),
                "port {port} must be rejected"
            );
        }
    }

    #[test]
    fn load_device_file_falls_back_to_filename_for_name() {
        let dir = TempDir::new("filename");
        let path = dir.path("from-filename.json");
        fs::write(
            &path,
            r#"{"ssh_public_key": "ssh-rsa AAAA", "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
        )
        .unwrap();

        let m = load_device_file(&path).expect("name must fall back to filename");
        assert_eq!(m.name.as_str(), "from-filename");
        assert_eq!(m.user, "");
        assert_eq!(m.port, 22);
        assert!(!m.port_explicit);
    }

    #[test]
    fn load_device_file_rejects_placeholder_and_localhost() {
        let dir = TempDir::new("invalid");
        let cases = [
            (
                r#"{"name": "x", "ssh_public_key": "", "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
                "empty key",
            ),
            (
                r#"{"name": "localhost", "ssh_public_key": "ssh-rsa AAAA", "signing_public_key": "MCowBQYDK2VwAyEA=="}"#,
                "localhost",
            ),
        ];
        for (i, (json, why)) in cases.iter().enumerate() {
            let path = dir.path(format!("c{i}.json").as_str());
            fs::write(&path, json).unwrap();
            assert!(load_device_file(&path).is_err(), "must reject {why}");
        }
    }

    #[test]
    fn save_device_file_roundtrip() {
        let dir = TempDir::new("roundtrip");
        let path = dir.path("roundtrip.json");
        let machine = Machine {
            name: DeviceName::new("roundtrip".to_string()).unwrap(),
            ssh_public_key: SshPublicKey::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFake"),
            signing_public_key: SigningPublicKeyB64::from("MCowBQYDK2VwAyEA=="),
            user: "bob".to_string(),
            port: 2222,
            port_explicit: true,
        };

        save_device_file(&path, &machine).expect("save must succeed");
        let loaded = load_device_file(&path).expect("saved device must load back");
        assert_eq!(loaded.name.as_str(), machine.name.as_str());
        assert_eq!(loaded.user, machine.user);
        assert_eq!(loaded.port, machine.port);
        assert!(loaded.port_explicit);
        assert_eq!(
            loaded.ssh_public_key.as_str(),
            machine.ssh_public_key.as_str()
        );
        assert_eq!(
            loaded.signing_public_key.as_str(),
            machine.signing_public_key.as_str()
        );
    }
}
