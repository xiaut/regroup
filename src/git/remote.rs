//! Git remote management
//!
//! This module handles git remote operations.

use std::io;
use std::path::Path;
use std::process::Command;

use crate::config::{CONFIG_DIR, GIT_DIR, Device};

// Git commands run through crate::ui::run_logged, which puts their output
// into the one log stream (see the comment in sync.rs).

/// Get the URL of an existing remote, or None if it doesn't exist
fn git_get_remote_url(repo_path: &Path, remote_name: &str) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .arg("get-url")
        .arg(remote_name)
        .output();

    match output {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => None,
    }
}

/// Add a git remote for a device (updates the URL if it already exists)
pub fn git_add_remote(repo_path: &Path, remote_name: &str, remote_url: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    match git_get_remote_url(repo_path, remote_name) {
        Some(existing_url) => {
            if existing_url != remote_url {
                // Update existing remote
                crate::ui::log(&format!(
                    "Updating git remote '{}' (may prompt for authentication)...",
                    remote_name
                ));
                let mut cmd = Command::new("git");
                cmd.current_dir(repo_path)
                    .arg("remote")
                    .arg("set-url")
                    .arg(remote_name)
                    .arg(remote_url);
                let output = crate::ui::run_logged(cmd)?;
                if !output.status.success() {
                    return Err(io::Error::other("failed to update remote"));
                }
            }
            Ok(())
        }
        None => {
            // Add new remote
            crate::ui::log(&format!(
                "Adding git remote '{}' (may prompt for authentication)...",
                remote_name
            ));
            let mut cmd = Command::new("git");
            cmd.current_dir(repo_path)
                .arg("remote")
                .arg("add")
                .arg(remote_name)
                .arg(remote_url);
            let output = crate::ui::run_logged(cmd)?;
            if !output.status.success() {
                return Err(io::Error::other("failed to add remote"));
            }
            Ok(())
        }
    }
}

/// Remove a git remote
pub fn git_remove_remote(repo_path: &Path, remote_name: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    crate::ui::log(&format!(
        "Removing git remote '{}' (may prompt for authentication)...",
        remote_name
    ));
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("remote")
        .arg("remove")
        .arg(remote_name);
    let output = crate::ui::run_logged(cmd)?;

    if !output.status.success() {
        return Err(io::Error::other("failed to remove remote"));
    }

    Ok(())
}

/// Get the list of existing git remotes
pub fn git_list_remotes(repo_path: &Path) -> Vec<String> {
    if !super::repo::is_git_repo(repo_path) {
        return Vec::new();
    }

    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// The git remote URL of a device for a given SSH user:
/// `{user}@{device}:~/{CONFIG_DIR}/{GIT_DIR}` — the device name is the
/// ~/.ssh/config alias regroup writes itself, never the IP (which changes
/// with the network).
pub fn remote_url(user: &str, device_name: &str) -> String {
    format!("{}@{}:~/{}/{}", user, device_name, CONFIG_DIR, GIT_DIR)
}

/// Sync git remotes to match all devices except self
pub fn sync_git_remotes(
    repo_path: &Path,
    devices: &[Device],
    local_hostname: &str,
    ssh_users: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let existing_remotes = git_list_remotes(repo_path);

    // Remove remotes that don't correspond to any device or are self
    for remote in &existing_remotes {
        if remote == local_hostname {
            // Never add self as a remote
            if let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove self remote {}: {}", remote, e));
            }
            continue;
        }

        let has_device = devices.iter().any(|m| m.name.as_str() == *remote);
        if !has_device {
            // Only remove remotes pointing at a regroup config repository
            // (stale device remotes); leave remotes the user configured for
            // other purposes (e.g. a backup repo) untouched.
            let is_regroup_remote =
                git_get_remote_url(repo_path, remote).is_none_or(|url| url.contains(CONFIG_DIR));
            if is_regroup_remote && let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove stale remote {}: {}", remote, e));
            }
        }
    }

    // Add remotes for all devices (except self)
    for device in devices {
        if device.name.as_str() == local_hostname {
            continue; // Skip self
        }

        // The remote URL needs an SSH user. Users are configuration, never
        // announced: the caller resolves each device's user (a temporary one
        // added this run, or one the device declared in its users/ directory).
        let Some(user) = ssh_users.get(device.name.as_str()) else {
            errors.push(format!(
                "no user known on {} — add a temporary user (Users panel) to sync with it",
                device.name.as_str()
            ));
            continue;
        };

        // Construct SSH URL for the device's config git repository (using hostname, not IP)
        let url = remote_url(user, device.name.as_str());

        if let Err(e) = git_add_remote(repo_path, device.name.as_str(), &url) {
            errors.push(format!("failed to add remote for {}: {}", device.name, e));
        }
    }

    errors
}
