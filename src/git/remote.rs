//! Git remote management
//!
//! This module handles git remote operations.

use std::io;
use std::path::Path;
use std::process::Command;

use crate::config::{CONFIG_DIR, GIT_DIR, Machine};

/// Add a git remote for a machine
pub fn git_add_remote(repo_path: &Path, remote_name: &str, remote_url: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }
    
    // Check if remote already exists
    let check = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .arg("get-url")
        .arg(remote_name)
        .output();
    
    match check {
        Ok(o) if o.status.success() => {
            let existing_url = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if existing_url != remote_url {
                // Update existing remote
                let status = Command::new("git")
                    .current_dir(repo_path)
                    .arg("remote")
                    .arg("set-url")
                    .arg(remote_name)
                    .arg(remote_url)
                    .status()?;
                if !status.success() {
                    return Err(io::Error::new(io::ErrorKind::Other, "failed to update remote"));
                }
            }
            Ok(())
        }
        _ => {
            // Add new remote
            let status = Command::new("git")
                .current_dir(repo_path)
                .arg("remote")
                .arg("add")
                .arg(remote_name)
                .arg(remote_url)
                .status()?;
            if !status.success() {
                return Err(io::Error::new(io::ErrorKind::Other, "failed to add remote"));
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
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .arg("remove")
        .arg(remote_name)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "failed to remove remote"));
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
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Refresh git remotes to match all machines except self
pub fn refresh_git_remotes(repo_path: &Path, machines: &[Machine], local_hostname: &str) -> Vec<String> {
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
            if let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove stale remote {}: {}", remote, e));
            }
        }
    }
    
    // Add remotes for all machines (except self)
    for machine in machines {
        if machine.name == local_hostname {
            continue; // Skip self
        }
        
        // A remote URL without a user is invalid: skip with an error so the
        // caller knows this machine cannot be synced yet
        if machine.user.is_empty() {
            errors.push(format!(
                "no SSH user configured for {} — set \"user\" in its device file to sync with it",
                machine.name
            ));
            continue;
        }
        
        // Construct SSH URL for the machine's config git repository (using hostname, not IP)
        let remote_url = format!("{}@{}:~/{}/{}", machine.user, machine.name, CONFIG_DIR, GIT_DIR);
        
        if let Err(e) = git_add_remote(repo_path, &machine.name, &remote_url) {
            errors.push(format!("failed to add remote for {}: {}", machine.name, e));
        }
    }
    
    errors
}
