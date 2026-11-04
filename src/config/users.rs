//! Per-device user files
//!
//! `devices/<name>/users/<user>.json` holds one file for each user known to
//! run on the device. Following the storage ownership rule, a device only
//! ever writes inside its own directory: the local device records its own
//! users (one file is created the first time a user runs regroup), while
//! other devices' user files arrive through git sync and are read-only
//! here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{device_dir, USERS_DIR};

/// A user known to run on a machine
#[derive(Clone, Debug, PartialEq)]
pub struct MachineUser {
    /// Login name (file name inside `users/`).
    pub name: String,
    /// RFC3339 timestamp of the first time this user ran regroup here.
    pub first_seen: String,
}

impl MachineUser {
    pub fn new(name: &str, first_seen: &str) -> Self {
        Self {
            name: name.to_string(),
            first_seen: first_seen.to_string(),
        }
    }
}

/// Path of a user file: `devices/<machine>/users/<sanitized user>.json`.
pub fn user_file_path(config_dir: &Path, machine: &str, user: &str) -> PathBuf {
    device_dir(config_dir, machine)
        .join(USERS_DIR)
        .join(format!("{}.json", sanitize(user)))
}

/// Characters that would break the file-name conventions ('@' qualifies a
/// remote member in connection names, '/' separates paths).
fn sanitize(name: &str) -> String {
    name.replace('@', "_").replace('/', "_").replace(' ', "_")
}

/// Create the user file on the local machine if it does not exist yet.
///
/// Returns true when the file was created (the caller commits it), false
/// when it already existed. Existing files are never rewritten: a user
/// entry is created once and then left alone.
pub fn ensure_local_user_file(config_dir: &Path, machine: &str, user: &str) -> io::Result<bool> {
    let path = user_file_path(config_dir, machine, user);
    if path.exists() {
        return Ok(false);
    }
    let entry = MachineUser::new(user, &chrono::Utc::now().to_rfc3339());
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("user file path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let value = serde_json::json!({
        "name": entry.name,
        "first_seen": entry.first_seen,
    });
    fs::write(&path, serde_json::to_string_pretty(&value)?)?;
    Ok(true)
}

/// Load all users known on a machine, sorted by name. Files that fail to
/// parse are skipped by the caller's warning policy (see load_config).
#[allow(
    dead_code,
    reason = "used by the upcoming per-user connection procedures; remote machines' user lists arrive through git sync"
)]
pub fn load_machine_users(config_dir: &Path, machine: &str) -> io::Result<Vec<MachineUser>> {
    let dir = device_dir(config_dir, machine).join(USERS_DIR);
    let mut users = Vec::new();
    if !dir.exists() {
        return Ok(users);
    }
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(name) = root.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let first_seen = root
            .get("first_seen")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        users.push(MachineUser::new(name, &first_seen));
    }
    users.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(users)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("regroup-users-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn ensure_creates_once_then_is_no_op() {
        let dir = temp_dir("once");
        assert!(ensure_local_user_file(&dir, "ordi", "alice").unwrap());
        assert!(!ensure_local_user_file(&dir, "ordi", "alice").unwrap());
        let users = load_machine_users(&dir, "ordi").unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "alice");
        assert!(!users[0].first_seen.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_file_names_are_sanitized() {
        let dir = temp_dir("sanitize");
        assert!(ensure_local_user_file(&dir, "ordi", "we ird@user/x").unwrap());
        assert!(dir
            .join("devices/ordi/users")
            .join("we_ird_user_x.json")
            .exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn users_sorted_by_name_and_missing_dir_is_empty() {
        let dir = temp_dir("sorted");
        assert!(ensure_local_user_file(&dir, "ordi", "bob").unwrap());
        assert!(ensure_local_user_file(&dir, "ordi", "alice").unwrap());
        let users = load_machine_users(&dir, "ordi").unwrap();
        assert_eq!(users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["alice", "bob"]);
        assert!(load_machine_users(&dir, "laptop").unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
