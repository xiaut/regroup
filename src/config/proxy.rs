//! Configuration proxy: the `tmp/` overlay.
//!
//! All configuration access goes through this proxy, which merges two
//! layers under the root configuration directory (`~/.regroup-config/`):
//!
//! - `git/` — the git repository: source of truth, synced with peers
//! - `tmp/` — a plain directory, a sibling of `git/` and never inside the
//!   repository (so it can never be committed, pushed or pulled), holding
//!   temporary, not-yet-authoritative data: beacon-derived placeholder
//!   `device.json` files and temporary users
//!
//! `tmp/` mirrors the `git/` layout (`tmp/devices/<name>/device.json`,
//! `tmp/devices/<name>/users/<user>.json`, ...).
//!
//! Read rule (per file): `effective = git/ ∪ (tmp/ − git/)`. On overlap
//! `git/` wins and the stale `tmp/` copy is removed — lazily on read
//! (self-healing) and eagerly after each successful git sync
//! ([`purge_tmp_overlaps`]). `tmp/` therefore only ever contributes paths
//! that do not exist in `git/` yet; it can never override synced data.
//!
//! Delete rule: removals apply to both layers, otherwise a stale `tmp/`
//! placeholder would resurrect a removed device on the next read.
//!
//! All state lives on disk in the two layers; in-memory structures are only
//! a cache of this merged view. Proxy operations run under a single lock: a
//! read snapshots both layers and the lazy purge runs under the same lock
//! as the read that triggers it (regroup is multi-threaded: UI + beacon
//! listener).
//!
//! The overlay is mandatory: tmp writes re-create `tmp/` when it vanished
//! mid-run (and restore mode 0700) and otherwise fail loudly — the caller
//! crashes with the reason.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::device::save_device_file;
use super::users::{sanitize_user, DeviceUser};
use super::{git_layer, tmp_layer, Device, DEVICES_DIR, DEVICE_FILE, USERS_DIR};

static PROXY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// The single proxy lock: held for every proxy operation (reads snapshot
/// both layers; the lazy purge runs under the same lock as the read).
fn lock() -> MutexGuard<'static, ()> {
    PROXY_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ============================================================
// Layer paths
// ============================================================

/// Create the `tmp/` overlay directory with mode `0700` (it may hold
/// not-yet-verified data) and return its path.
pub fn ensure_tmp_dir(root: &Path) -> io::Result<PathBuf> {
    let dir = tmp_layer(root);
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: a restrictive mode on the overlay, never a reason
        // to fail.
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// Write `path` inside the `tmp/` overlay, making sure the overlay exists
/// first. Re-creating `tmp/` fixes transient `NotFound` errors (the
/// overlay was removed while the process was running), so the write is
/// retried once after re-creating it; any other error is returned to the
/// caller — the overlay is mandatory, the caller crashes with the reason.
fn write_tmp_file(root: &Path, path: &Path, write: impl Fn(&Path) -> io::Result<()>) -> io::Result<()> {
    ensure_tmp_dir(root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match write(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            ensure_tmp_dir(root)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            write(path)
        }
        result => result,
    }
}

/// Path of `rel` inside the git layer.
fn git_path(root: &Path, rel: &str) -> PathBuf {
    git_layer(root).join(rel)
}

/// Path of `rel` inside the tmp layer.
fn tmp_path(root: &Path, rel: &str) -> PathBuf {
    tmp_layer(root).join(rel)
}

// ============================================================
// Read rule: effective = git/ ∪ (tmp/ − git/)
// ============================================================

/// Resolve `rel` (a path relative to a layer root, e.g.
/// `devices/<name>/device.json`) through the proxy: the `git/` copy wins
/// and a stale `tmp/` copy is purged lazily; otherwise the `tmp/` copy is
/// returned. Returns None when the path exists in neither layer.
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let _guard = lock();
    let git = git_path(root, rel);
    if git.exists() {
        let tmp = tmp_path(root, rel);
        if tmp.exists() {
            // Stale: its authoritative counterpart is in git/.
            let _ = fs::remove_file(&tmp);
        }
        return Some(git);
    }
    let tmp = tmp_path(root, rel);
    if tmp.exists() { Some(tmp) } else { None }
}

/// Whether `rel` exists in the git (authoritative) layer.
#[allow(
    dead_code,
    reason = "part of the proxy API surface; exercised by the unit tests"
)]
pub fn in_git(root: &Path, rel: &str) -> bool {
    git_path(root, rel).exists()
}

/// One file of the merged view of `rel_dir`: its layer-relative path, its
/// effective path, and whether it exists only in the `tmp/` layer.
#[derive(Clone, Debug)]
pub struct MergedFile {
    pub rel: PathBuf,
    pub path: PathBuf,
    pub tmp_only: bool,
}

/// List every file under `rel_dir` (relative to a layer root) in the merged
/// view, sorted by relative path. On overlap `git/` wins and the stale
/// `tmp/` copy is purged.
pub fn merged_files(root: &Path, rel_dir: &str) -> Vec<MergedFile> {
    let _guard = lock();
    let mut out: Vec<MergedFile> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    collect_files(&git_layer(root).join(rel_dir), rel_dir, &mut |rel| {
        if seen.insert(rel.clone()) {
            out.push(MergedFile {
                path: git_path(root, rel.to_str().unwrap_or_default()),
                rel,
                tmp_only: false,
            });
        }
    });
    collect_files(&tmp_layer(root).join(rel_dir), rel_dir, &mut |rel| {
        if seen.insert(rel.clone()) {
            out.push(MergedFile {
                path: tmp_path(root, rel.to_str().unwrap_or_default()),
                rel,
                tmp_only: true,
            });
        } else {
            // Overlap: git/ wins, the tmp/ copy is stale.
            let _ = fs::remove_file(tmp_path(root, rel.to_str().unwrap_or_default()));
        }
    });

    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Recursively collect the layer-relative paths of all files under `dir`.
fn collect_files(dir: &Path, prefix: &str, push: &mut impl FnMut(PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let rel = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            if path.is_dir() {
                collect_files(&path, &rel, push);
            } else {
                push(PathBuf::from(rel));
            }
        }
    }
}

// ============================================================
// Purge rule
// ============================================================

/// Eager purge: remove every `tmp/` file whose relative path also exists in
/// `git/`. Per file, not per directory: `tmp/devices/X/` keeps its
/// non-overlapping files while `tmp/devices/X/device.json` is dropped once
/// `git/devices/X/device.json` exists. Returns the number of files purged.
///
/// Called after each successful git sync; reads additionally purge lazily
/// (see [`resolve`] and [`merged_files`]).
pub fn purge_tmp_overlaps(root: &Path) -> usize {
    let _guard = lock();
    let mut purged = 0;
    let tmp_root = tmp_layer(root);
    let git_root = git_layer(root);
    let tmp_root_for_closure = tmp_root.clone();
    collect_files(&tmp_root, "", &mut |rel| {
        let rel_str = rel.to_str().unwrap_or_default();
        if git_root.join(rel_str).exists()
            && fs::remove_file(tmp_root_for_closure.join(rel_str)).is_ok()
        {
            purged += 1;
        }
    });
    purged
}

// ============================================================
// Delete rule: both layers
// ============================================================

/// Delete `rel` from both layers. Returns whether anything was removed.
pub fn delete_rel(root: &Path, rel: &str) -> io::Result<bool> {
    let _guard = lock();
    let mut removed = false;
    for path in [git_path(root, rel), tmp_path(root, rel)] {
        match fs::remove_file(&path) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

/// Delete a device's whole directory from both layers. Returns whether the
/// `git/` copy existed (the caller commits that deletion so it propagates
/// to peers on sync).
pub fn delete_device(root: &Path, name: &str) -> io::Result<bool> {
    let _guard = lock();
    let rel = format!("{DEVICES_DIR}/{name}");
    let mut existed_in_git = false;
    for (layer, is_git) in [(git_layer(root), true), (tmp_layer(root), false)] {
        let dir = layer.join(&rel);
        match fs::remove_dir_all(&dir) {
            Ok(()) => existed_in_git |= is_git,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(existed_in_git)
}

// ============================================================
// Device helpers
// ============================================================

/// Names of all devices visible through the proxy: every device directory
/// (in either layer) that has an effective `device.json`.
pub fn merged_device_names(root: &Path) -> Vec<String> {
    let mut names: HashSet<String> = HashSet::new();
    for layer in [git_layer(root), tmp_layer(root)] {
        let devices = layer.join(DEVICES_DIR);
        if let Ok(entries) = fs::read_dir(&devices) {
            for entry in entries.flatten() {
                if entry.path().is_dir()
                    && let Some(name) = entry.file_name().to_str()
                {
                    names.insert(name.to_string());
                }
            }
        }
    }
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|name| device_file(root, name).is_some())
        .collect();
    names.sort();
    names
}

/// Effective path of a device's `device.json` through the proxy (git/ wins
/// over a tmp/ placeholder), if any.
pub fn device_file(root: &Path, name: &str) -> Option<PathBuf> {
    resolve(root, &format!("{DEVICES_DIR}/{name}/{DEVICE_FILE}"))
}

/// Path of a device's placeholder `device.json` in the tmp/ overlay.
pub fn tmp_device_file(root: &Path, name: &str) -> PathBuf {
    tmp_path(root, &format!("{DEVICES_DIR}/{name}/{DEVICE_FILE}"))
}

/// Write a device's placeholder `device.json` into the tmp/ overlay. Never
/// committed or synced; purged automatically once the device's
/// authoritative `device.json` arrives in `git/` through a sync. Returns
/// the path written.
pub fn write_tmp_device_file(root: &Path, name: &str, device: &Device) -> io::Result<PathBuf> {
    let path = tmp_device_file(root, name);
    write_tmp_file(root, &path, |p| save_device_file(p, device))?;
    Ok(path)
}

// ============================================================
// User helpers
// ============================================================

fn user_rel(device: &str, user: &str) -> String {
    format!(
        "{DEVICES_DIR}/{device}/{USERS_DIR}/{}.json",
        sanitize_user(user)
    )
}

/// Effective path of a user file through the proxy (git/ wins over a
/// temporary tmp/ copy), if any.
#[allow(
    dead_code,
    reason = "part of the proxy API surface; exercised by the unit tests"
)]
pub fn user_file(root: &Path, device: &str, user: &str) -> Option<PathBuf> {
    resolve(root, &user_rel(device, user))
}

/// Path of a user file in the tmp/ overlay.
pub fn tmp_user_file(root: &Path, device: &str, user: &str) -> PathBuf {
    tmp_path(root, &user_rel(device, user))
}

/// Persist a temporary user in the tmp/ overlay. Purged automatically once
/// the device declares the same user in `git/`. Returns the path written.
pub fn write_tmp_user_file(root: &Path, device: &str, user: &str) -> io::Result<PathBuf> {
    let path = tmp_user_file(root, device, user);
    let entry = DeviceUser::new(user, &chrono::Utc::now().to_rfc3339());
    let value = serde_json::json!({
        "name": entry.name,
        "first_seen": entry.first_seen,
    });
    let text = serde_json::to_string_pretty(&value)?;
    write_tmp_file(root, &path, |p| fs::write(p, &text))?;
    Ok(path)
}

/// Delete a user file from both layers. Returns whether anything was removed.
pub fn delete_user(root: &Path, device: &str, user: &str) -> io::Result<bool> {
    delete_rel(root, &user_rel(device, user))
}

#[cfg(test)]
mod tests {
    use super::super::TMP_DIR;
    use super::*;
    use crate::config::DeviceName;
    use crate::ssh::keys::{SigningPublicKeyB64, SshPublicKey};

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("regroup-proxy-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn sample_device(name: &str) -> Device {
        Device {
            name: DeviceName::new(name.to_string()).unwrap(),
            ssh_public_key: SshPublicKey::from("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFake"),
            signing_public_key: SigningPublicKeyB64::from("MCowBQYDK2VwAyEA=="),
            port: 22,
            port_explicit: false,
        }
    }

    #[test]
    fn tmp_dir_is_created_with_mode_0700() {
        let root = temp_root("perms");
        let dir = ensure_tmp_dir(&root).unwrap();
        assert_eq!(dir, root.join(TMP_DIR));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "tmp/ must be 0700");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_prefers_git_and_purges_stale_tmp_copy() {
        let root = temp_root("resolve");
        let rel = "devices/laptop/device.json";
        // Only in tmp/ first.
        fs::create_dir_all(root.join("tmp/devices/laptop")).unwrap();
        fs::write(root.join("tmp").join(rel), "tmp").unwrap();
        let resolved = resolve(&root, rel).unwrap();
        assert!(resolved.starts_with(root.join("tmp")), "tmp-only resolves to tmp: {resolved:?}");

        // Authoritative copy arrives in git/: git/ wins, tmp/ copy purged.
        fs::create_dir_all(root.join("git/devices/laptop")).unwrap();
        fs::write(root.join("git").join(rel), "git").unwrap();
        let resolved = resolve(&root, rel).unwrap();
        assert!(resolved.starts_with(root.join("git")), "git wins: {resolved:?}");
        assert!(!root.join("tmp").join(rel).exists(), "stale tmp/ copy must be purged");
        assert!(in_git(&root, rel));

        // Neither layer: None.
        assert!(resolve(&root, "devices/ghost/device.json").is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merged_files_unions_both_layers_and_purges_overlaps() {
        let root = temp_root("merged");
        fs::create_dir_all(root.join("git/devices/a/users")).unwrap();
        fs::create_dir_all(root.join("tmp/devices/a/users")).unwrap();
        fs::create_dir_all(root.join("tmp/devices/b")).unwrap();
        fs::write(root.join("git/devices/a/users/alice.json"), "a").unwrap();
        fs::write(root.join("git/devices/a/device.json"), "a").unwrap();
        // Overlap: present in both, git/ wins and tmp/ copy is purged.
        fs::write(root.join("tmp/devices/a/users/alice.json"), "stale").unwrap();
        // tmp-only entries.
        fs::write(root.join("tmp/devices/a/users/bob.json"), "b").unwrap();
        fs::write(root.join("tmp/devices/b/device.json"), "b").unwrap();

        let files = merged_files(&root, DEVICES_DIR);
        let rels: Vec<&str> = files.iter().map(|f| f.rel.to_str().unwrap()).collect();
        assert_eq!(
            rels,
            [
                "devices/a/device.json",
                "devices/a/users/alice.json",
                "devices/a/users/bob.json",
                "devices/b/device.json",
            ]
        );
        assert!(!root.join("tmp/devices/a/users/alice.json").exists(), "overlap purged");
        let alice = files.iter().find(|f| f.rel.ends_with("alice.json")).unwrap();
        assert!(!alice.tmp_only && alice.path.starts_with(root.join("git")));
        let bob = files.iter().find(|f| f.rel.ends_with("bob.json")).unwrap();
        assert!(bob.tmp_only && bob.path.starts_with(root.join("tmp")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn purge_tmp_overlaps_is_per_file() {
        let root = temp_root("purge");
        fs::create_dir_all(root.join("git/devices/x/users")).unwrap();
        fs::create_dir_all(root.join("tmp/devices/x/users")).unwrap();
        fs::write(root.join("git/devices/x/device.json"), "g").unwrap();
        fs::write(root.join("git/devices/x/users/alice.json"), "g").unwrap();
        fs::write(root.join("tmp/devices/x/device.json"), "stale").unwrap();
        fs::write(root.join("tmp/devices/x/users/alice.json"), "stale").unwrap();
        fs::write(root.join("tmp/devices/x/users/bob.json"), "keep").unwrap();

        assert_eq!(purge_tmp_overlaps(&root), 2);
        assert!(!root.join("tmp/devices/x/device.json").exists());
        assert!(!root.join("tmp/devices/x/users/alice.json").exists());
        assert!(root.join("tmp/devices/x/users/bob.json").exists(), "non-overlapping file kept");
        // Idempotent.
        assert_eq!(purge_tmp_overlaps(&root), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_hits_both_layers() {
        let root = temp_root("delete");
        fs::create_dir_all(root.join("git/devices/x/users")).unwrap();
        fs::create_dir_all(root.join("tmp/devices/x")).unwrap();
        fs::write(root.join("git/devices/x/device.json"), "g").unwrap();
        fs::write(root.join("git/devices/x/users/alice.json"), "g").unwrap();
        fs::write(root.join("tmp/devices/x/device.json"), "t").unwrap();

        assert!(delete_device(&root, "x").unwrap(), "git/ copy existed");
        assert!(!root.join("git/devices/x").exists());
        assert!(!root.join("tmp/devices/x").exists());
        // Deleting again: nothing existed, no error.
        assert!(!delete_device(&root, "x").unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tmp_device_and_user_files_roundtrip_through_the_proxy() {
        let root = temp_root("roundtrip");
        ensure_tmp_dir(&root).unwrap();

        let device = sample_device("laptop");
        let path = write_tmp_device_file(&root, "laptop", &device).unwrap();
        assert!(path.starts_with(root.join("tmp")), "placeholder goes to tmp/: {path:?}");
        assert!(!root.join("git").join("devices/laptop/device.json").exists(), "never in git/");

        // Visible through the proxy as a device.
        assert_eq!(merged_device_names(&root), vec!["laptop".to_string()]);
        let effective = device_file(&root, "laptop").unwrap();
        assert_eq!(effective, path);
        let loaded = super::super::device::load_device_file(&effective).unwrap();
        assert_eq!(loaded.name.as_str(), "laptop");

        // Temp user: visible as tmp-only, then purged once declared in git/.
        write_tmp_user_file(&root, "laptop", "alice").unwrap();
        assert!(user_file(&root, "laptop", "alice").unwrap().starts_with(root.join("tmp")));
        assert!(delete_user(&root, "laptop", "alice").unwrap());
        assert!(user_file(&root, "laptop", "alice").is_none());

        // Simulate the sync bringing the authoritative files: git/ wins and
        // the tmp/ placeholder is purged.
        fs::create_dir_all(root.join("git/devices/laptop")).unwrap();
        let git_device = root.join("git/devices/laptop/device.json");
        save_device_file(&git_device, &sample_device("laptop")).unwrap();
        let effective = device_file(&root, "laptop").unwrap();
        assert_eq!(effective, git_device, "git/ wins once present");
        assert!(!path.exists(), "stale tmp/ placeholder purged on read");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tmp_write_recreates_a_removed_overlay() {
        let root = temp_root("recreate");
        ensure_tmp_dir(&root).unwrap();
        write_tmp_user_file(&root, "laptop", "alice").unwrap();

        // The overlay vanished mid-run (e.g. another instance wiped the
        // config): the write must re-create it, with mode 0700, and succeed.
        fs::remove_dir_all(root.join(TMP_DIR)).unwrap();
        write_tmp_user_file(&root, "laptop", "bob").unwrap();
        let dir = root.join(TMP_DIR);
        assert!(dir.is_dir(), "overlay re-created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "re-created overlay must be 0700");
        }
        assert!(root.join("tmp/devices/laptop/users/bob.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tmp_write_error_propagates_when_the_overlay_cannot_exist() {
        let root = temp_root("blocked");
        // `tmp` exists as a regular file: the overlay can neither be created
        // nor written — the error must reach the caller (which crashes with
        // the reason) instead of being swallowed.
        fs::write(root.join(TMP_DIR), "not a directory").unwrap();
        assert!(write_tmp_user_file(&root, "laptop", "alice").is_err());
        assert!(write_tmp_device_file(&root, "laptop", &sample_device("laptop")).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
