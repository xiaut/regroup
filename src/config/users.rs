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

use super::{device_dir, DEVICES_DIR, USERS_DIR};

/// A user known to run on a device
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceUser {
    /// Login name (file name inside `users/`).
    pub name: String,
    /// RFC3339 timestamp of the first time this user ran regroup here.
    pub first_seen: String,
}

impl DeviceUser {
    pub fn new(name: &str, first_seen: &str) -> Self {
        Self {
            name: name.to_string(),
            first_seen: first_seen.to_string(),
        }
    }
}

/// Path of a user file in the git layer:
/// `devices/<device>/users/<sanitized user>.json`.
pub fn user_file_path(config_dir: &Path, device: &str, user: &str) -> PathBuf {
    device_dir(config_dir, device)
        .join(USERS_DIR)
        .join(format!("{}.json", sanitize_user(user)))
}

/// Characters that would break the file-name conventions ('@' qualifies a
/// remote member in connection names, '/' separates paths).
pub(crate) fn sanitize_user(name: &str) -> String {
    name.replace('@', "_").replace('/', "_").replace(' ', "_")
}

/// Create the user file on the local device if it does not exist yet.
///
/// Returns true when the file was created (the caller commits it), false
/// when it already existed. Existing files are never rewritten: a user
/// entry is created once and then left alone.
pub fn ensure_local_user_file(config_dir: &Path, device: &str, user: &str) -> io::Result<bool> {
    let path = user_file_path(config_dir, device, user);
    if path.exists() {
        return Ok(false);
    }
    let entry = DeviceUser::new(user, &chrono::Utc::now().to_rfc3339());
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

/// Load all users known on a device, sorted by name. Files that fail to
/// parse are skipped by the caller's warning policy (see load_config).
#[allow(
    dead_code,
    reason = "used by the upcoming per-user connection procedures; remote devices' user lists arrive through git sync"
)]
pub fn load_device_users(config_dir: &Path, device: &str) -> io::Result<Vec<DeviceUser>> {
    let dir = device_dir(config_dir, device).join(USERS_DIR);
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
        users.push(DeviceUser::new(name, &first_seen));
    }
    users.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(users)
}

/// A user visible through the configuration proxy, with the layer it comes
/// from: `temporary` is true when the file exists only in the `tmp/` overlay
/// (not yet declared by the device itself in `git/`).
#[derive(Clone, Debug, PartialEq)]
pub struct MergedUser {
    pub user: DeviceUser,
    pub temporary: bool,
}

/// Load all users of a device through the proxy (`git/ ∪ (tmp/ − git/)`,
/// `git/` wins on overlap, stale `tmp/` copies purged), sorted by name.
/// Unparseable files are skipped, like in [`load_device_users`].
pub fn load_device_users_merged(root: &Path, device: &str) -> io::Result<Vec<MergedUser>> {
    let rel_dir = format!("{DEVICES_DIR}/{device}/{USERS_DIR}");
    let mut users = Vec::new();
    for file in super::proxy::merged_files(root, &rel_dir) {
        if file.path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&file.path) else {
            continue;
        };
        let Ok(root_json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(name) = root_json.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let first_seen = root_json
            .get("first_seen")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        users.push(MergedUser {
            user: DeviceUser::new(name, &first_seen),
            temporary: file.tmp_only,
        });
    }
    users.sort_by(|a, b| a.user.name.cmp(&b.user.name));
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
        let users = load_device_users(&dir, "ordi").unwrap();
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
        let users = load_device_users(&dir, "ordi").unwrap();
        assert_eq!(users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["alice", "bob"]);
        assert!(load_device_users(&dir, "laptop").unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merged_users_union_layers_with_git_winning() {
        use super::super::proxy;
        // root layout: root/git/... (declared) and root/tmp/... (temporary)
        let root = temp_dir("merged");
        let git = root.join("git");
        // Declared in git/.
        assert!(ensure_local_user_file(&git, "ordi", "alice").unwrap());
        // Temporary in tmp/ only.
        proxy::write_tmp_user_file(&root, "ordi", "bob").unwrap();
        // Overlap: same user in both layers — git/ wins, tmp/ copy purged.
        proxy::write_tmp_user_file(&root, "ordi", "alice").unwrap();

        let users = load_device_users_merged(&root, "ordi").unwrap();
        let names: Vec<(&str, bool)> = users.iter().map(|u| (u.user.name.as_str(), u.temporary)).collect();
        assert_eq!(names, [("alice", false), ("bob", true)]);
        assert!(!proxy::tmp_user_file(&root, "ordi", "alice").exists(), "stale tmp/ copy purged");

        // Once the device declares bob in git/, the tmp/ copy is stale.
        assert!(ensure_local_user_file(&git, "ordi", "bob").unwrap());
        let users = load_device_users_merged(&root, "ordi").unwrap();
        assert!(users.iter().all(|u| !u.temporary));
        let _ = fs::remove_dir_all(&root);
    }
}
