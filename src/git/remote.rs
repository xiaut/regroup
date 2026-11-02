//! Git remote management
//!
//! This module handles git remote operations.

use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{CONFIG_DIR, GIT_DIR, Machine};

/// Run a git command quietly: its output would corrupt the interactive
/// display (which owns the terminal), and failures are reported through the
/// returned errors.
fn quiet(mut cmd: Command) -> Command {
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd
}

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

/// Add a git remote for a machine (updates the URL if it already exists)
pub fn git_add_remote(repo_path: &Path, remote_name: &str, remote_url: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    match git_get_remote_url(repo_path, remote_name) {
        Some(existing_url) => {
            if existing_url != remote_url {
                // Update existing remote
                let status = quiet(Command::new("git"))
                    .current_dir(repo_path)
                    .arg("remote")
                    .arg("set-url")
                    .arg(remote_name)
                    .arg(remote_url)
                    .status()?;
                if !status.success() {
                    return Err(io::Error::other("failed to update remote"));
                }
            }
            Ok(())
        }
        None => {
            // Add new remote
            let status = quiet(Command::new("git"))
                .current_dir(repo_path)
                .arg("remote")
                .arg("add")
                .arg(remote_name)
                .arg(remote_url)
                .status()?;
            if !status.success() {
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

    let status = quiet(Command::new("git"))
        .current_dir(repo_path)
        .arg("remote")
        .arg("remove")
        .arg(remote_name)
        .status()?;

    if !status.success() {
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

/// Refresh git remotes to match all machines except self
pub fn refresh_git_remotes(
    repo_path: &Path,
    machines: &[Machine],
    local_hostname: &str,
) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let existing_remotes = git_list_remotes(repo_path);

    // Remove remotes that don't correspond to any machine or are self
    for remote in &existing_remotes {
        if remote == local_hostname {
            // Never add self as a remote
            if let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove self remote {}: {}", remote, e));
            }
            continue;
        }

        let has_machine = machines.iter().any(|m| m.name == *remote);
        if !has_machine {
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

    // Add remotes for all machines (except self)
    for machine in machines {
        if machine.name == local_hostname {
            continue; // Skip self
        }

        // The remote URL needs an SSH user. Devices discovered via beacon carry
        // none (the beacon format has no user field), so fall back to the
        // current local user; "user" in the device file overrides it.
        let user = if machine.user.is_empty() {
            env::var("USER")
                .or_else(|_| env::var("LOGNAME"))
                .unwrap_or_default()
        } else {
            machine.user.clone()
        };
        if user.is_empty() {
            errors.push(format!(
                "no SSH user configured for {} — set \"user\" in its device file to sync with it",
                machine.name
            ));
            continue;
        }

        // Construct SSH URL for the machine's config git repository (using hostname, not IP)
        let remote_url = format!("{}@{}:~/{}/{}", user, machine.name, CONFIG_DIR, GIT_DIR);

        if let Err(e) = git_add_remote(repo_path, &machine.name, &remote_url) {
            errors.push(format!("failed to add remote for {}: {}", machine.name, e));
        }
    }

    errors
}
