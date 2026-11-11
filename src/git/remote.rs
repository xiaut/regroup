//! Git remote management
//!
//! This module handles git remote operations.

use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{CONFIG_DIR, GIT_DIR, Machine};

/// Run a git command and log its output.
/// When in direct output mode (temporary CLI mode), stderr is passed through
/// directly to allow interactive prompts (e.g., password) to work.
fn quiet(mut cmd: Command) -> io::Result<std::process::Output> {
    // Check if we're in direct output mode (temporary CLI mode from TUI)
    let direct_mode = crate::ui::logging::is_direct_output_mode();
    
    // In direct mode, pass stderr through to terminal for interactive prompts
    // In normal mode, capture stderr to log it
    let stderr_config = if direct_mode {
        Stdio::inherit()  // Pass through to terminal
    } else {
        Stdio::piped()    // Capture for logging
    };
    
    // Always capture stdout
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(stderr_config)
        .output()?;
    
    // Log stdout
    if !output.stdout.is_empty() {
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        for line in stdout_str.lines() {
            if !line.is_empty() {
                crate::ui::log(line);
            }
        }
    }
    
    // In normal mode, stderr was captured - log it
    // In direct mode, stderr was passed through - don't try to log it
    if !direct_mode && !output.stderr.is_empty() {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        for line in stderr_str.lines() {
            if !line.is_empty() {
                crate::ui::log(line);
            }
        }
    }
    
    Ok(output)
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
                let output = quiet(cmd)?;
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
            let output = quiet(cmd)?;
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
    let output = quiet(cmd)?;

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

/// Sync git remotes to match all machines except self
pub fn sync_git_remotes(
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

        let has_machine = machines.iter().any(|m| m.name.as_str() == *remote);
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
        if machine.name.as_str() == local_hostname {
            continue; // Skip self
        }

        // The remote URL needs an SSH user. Devices discovered via beacon carry
        // their user field; fall back to the current local user if empty.
        // "user" in the device file overrides it.
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
                machine.name.as_str()
            ));
            continue;
        }

        // Construct SSH URL for the machine's config git repository (using hostname, not IP)
        let remote_url = format!("{}@{}:~/{}/{}", user, machine.name.as_str(), CONFIG_DIR, GIT_DIR);

        if let Err(e) = git_add_remote(repo_path, machine.name.as_str(), &remote_url) {
            errors.push(format!("failed to add remote for {}: {}", machine.name, e));
        }
    }

    errors
}
