//! Git operations module for regroup
//!
//! This module handles all git-related functionality including:
//! - Repository initialization and checks
//! - Remote management
//! - Pull/push operations
//! - Change detection

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Machine, DEVICES_DIR, GLOBAL_CONFIG_FILE};

// ============================================================
// Repository operations
// ============================================================

/// Check if a directory is a git repository
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Initialize a git repository
pub fn git_init(repo_path: &Path) -> io::Result<()> {
    if repo_path.exists() {
        return Ok(());
    }
    
    fs::create_dir_all(repo_path)?;
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("init")
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git init failed",
        ));
    }
    
    // Configure git to allow commits without user info
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.email")
        .arg("regroup@local")
        .status();
    
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.name")
        .arg("regroup")
        .status();
    
    // Configure git to allow pushing to the currently checked-out branch
    // This is essential for decentralized sync where each device pushes to others
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("receive.denyCurrentBranch")
        .arg("updateInstead")
        .status();
    
    // Configure pull to use rebase by default for cleaner history
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("pull.rebase")
        .arg("true")
        .status();
    
    Ok(())
}

// ============================================================
// Pull/Push operations
// ============================================================

/// Pull updates from all git remotes
pub fn git_pull_all(repo_path: &Path) -> Vec<String> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    // Check if there are any remotes configured
    let remotes = git_list_remotes(repo_path);
    if remotes.is_empty() {
        // No remotes configured - this is expected on first run
        return Vec::new();
    }
    
    // Inform user that we're about to pull from remotes (may require password)
    eprintln!("Syncing: Pulling updates from {} remote(s)...", remotes.len());
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("pull")
        .arg("--all")
        .arg("--rebase")
        .output();
    
    match output {
        Ok(o) if o.status.success() => Vec::new(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stderr_lower = stderr.to_lowercase();
            
            // Check for authentication errors (including password prompts)
            if stderr_lower.contains("permission denied") || 
               stderr_lower.contains("authentication failed") ||
               stderr_lower.contains("no suitable response") ||
               stderr_lower.contains("connection closed by") ||
               stderr_lower.contains("impossible de lire le dépôt distant") ||
               stderr_lower.contains("could not read from remote") ||
               stderr_lower.contains("repository not found") ||
               stderr_lower.contains("dépôt non trouvé") ||
               stderr_lower.contains("password") ||
               stderr_lower.contains("mot de passe") {
                vec![String::from(
                    "SSH authentication required. To sync with remote devices:\n  1. Set up passwordless SSH authentication (recommended): ssh-copy-id user@hostname\n  2. Or ensure your SSH agent has the required keys loaded\n  3. Check that the remote device is reachable and SSH is properly configured"
                )]
            } else if stderr.contains("fatal") || stderr.contains("error") {
                // Filter out SSH usage messages (they're just noise)
                if stderr.contains("usage: ssh") {
                    vec![String::from(
                        "Git sync error. Ensure remote devices are configured correctly."
                    )]
                } else {
                    vec![stderr.trim().to_string()]
                }
            } else {
                Vec::new()
            }
        }
        Err(e) => vec![format!("{} - To enable git sync, set up SSH authentication", e)],
    }
}

/// Push to all git remotes
pub fn git_push_all(repo_path: &Path) -> Vec<String> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    // Check if there are any remotes configured
    let remotes = git_list_remotes(repo_path);
    if remotes.is_empty() {
        // No remotes configured - this is expected on first run
        return Vec::new();
    }
    
    // Inform user that we're about to push to remotes (may require password)
    eprintln!("Syncing: Pushing to {} remote(s)...", remotes.len());
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("push")
        .arg("--all")
        .arg("--force-with-lease")
        .arg("--porcelain")
        .output();
    
    match output {
        Ok(o) if o.status.success() => Vec::new(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stderr_lower = stderr.to_lowercase();
            
            // Check for authentication errors
            if stderr_lower.contains("permission denied") || 
               stderr_lower.contains("authentication failed") ||
               stderr_lower.contains("no suitable response") ||
               stderr_lower.contains("connection closed by") ||
               stderr_lower.contains("impossible de lire le dépôt distant") ||
               stderr_lower.contains("could not read from remote") ||
               stderr_lower.contains("password") ||
               stderr_lower.contains("mot de passe") {
                vec![String::from(
                    "SSH authentication required. To sync with remote devices:\n  1. Set up passwordless SSH authentication (recommended): ssh-copy-id user@hostname\n  2. Or ensure your SSH agent has the required keys loaded\n  3. Check that the remote device is reachable and SSH is properly configured"
                )]
            } else if stderr_lower.contains("fatal") || stderr_lower.contains("error") {
                // Check if it's expected errors in a decentralized system
                // These are normal and shouldn't cause the program to fail
                if stderr_lower.contains("no remotes") || 
                   stderr_lower.contains("pas de destination pour pousser") ||
                   stderr_lower.contains("tip of your current branch is behind") ||
                   stderr_lower.contains("updates were rejected") ||
                   stderr_lower.contains("non-fast-forward") {
                    // These are expected in decentralized setups with one-way SSH
                    Vec::new()
                } else if stderr.contains("usage: ssh") {
                    // Filter out SSH usage messages
                    Vec::new()
                } else {
                    vec![stderr.trim().to_string()]
                }
            } else {
                Vec::new()
            }
        }
        Err(e) => vec![format!("{} - To enable git sync, set up SSH authentication", e)],
    }
}

// ============================================================
// Remote management
// ============================================================

/// Add a git remote for a machine
pub fn git_add_remote(repo_path: &Path, remote_name: &str, remote_url: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
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
    if !is_git_repo(repo_path) {
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
    if !is_git_repo(repo_path) {
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

// ============================================================
// Change detection
// ============================================================

/// Check if there are any unstaged changes in the git repository
/// Returns true if there are modified, added, or deleted files not yet committed
pub fn has_unstaged_changes(repo_path: &Path) -> bool {
    if !is_git_repo(repo_path) {
        return false;
    }
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("status")
        .arg("--porcelain")
        .output();
    
    match output {
        Ok(o) if o.status.success() => {
            let status = String::from_utf8_lossy(&o.stdout);
            !status.trim().is_empty()
        }
        _ => false,
    }
}

/// Get list of modified configuration files (settings.json and devices/*.json)
/// Returns paths relative to repo_path
pub fn get_modified_config_files(repo_path: &Path) -> Vec<PathBuf> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("status")
        .arg("--porcelain")
        .output();
    
    match output {
        Ok(o) if o.status.success() => {
            let status = String::from_utf8_lossy(&o.stdout);
            let mut modified_files = Vec::new();
            
            for line in status.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                // Porcelain format: "<index_status><workspace_status> <file>"
                // We're interested in modified (M) or added (A) files
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 2 {
                    continue;
                }
                let _file_status = parts[0];
                let file_path = parts[1];
                
                // Check if it's a config file we care about
                if file_path == GLOBAL_CONFIG_FILE || 
                   file_path.starts_with(&format!("{}/", DEVICES_DIR)) {
                    modified_files.push(PathBuf::from(file_path));
                }
            }
            modified_files
        }
        _ => Vec::new(),
    }
}

// ============================================================
// High-level operations
// ============================================================

/// Refresh git remotes to match all machines except self
pub fn refresh_git_remotes(repo_path: &Path, machines: &[Machine], local_hostname: &str) -> Vec<String> {
    let mut errors = Vec::new();
    
    if !is_git_repo(repo_path) {
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
        
        // Construct SSH URL for the machine's config repo using hostname (not IP)
        let remote_url = format!("{}@{}:~/.regroup-config", machine.user, machine.name);
        
        if let Err(e) = git_add_remote(repo_path, &machine.name, &remote_url) {
            errors.push(format!("failed to add remote for {}: {}", machine.name, e));
        }
    }
    
    errors
}

/// Sync with all remotes (pull then push to all remotes)
/// Each device A can both pull from and push to all other devices
pub fn sync_with_remotes(repo_path: &Path, has_local_changes: bool) -> Vec<String> {
    let mut errors = Vec::new();
    
    // Pull from all remotes - device A pulls from all other devices
    let pull_errors = git_pull_all(repo_path);
    errors.extend(pull_errors);
    
    // Push to all remotes - device A pushes to all other devices
    // Only push if we have local changes to share
    if has_local_changes {
        let push_errors = git_push_all(repo_path);
        errors.extend(push_errors);
    }
    
    errors
}

/// Git add and commit a specific file (for device files and shared files separately)
/// This ensures device-owned files are in separate commits from shared files,
/// preventing conflicts during decentralized sync.
pub fn git_add_and_commit_file(repo_path: &Path, file_path: &Path, message: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
        return Ok(());
    }
    
    // Add the specific file
    let relative_path = file_path.strip_prefix(repo_path).unwrap_or(file_path);
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("add")
        .arg(relative_path)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git add failed",
        ));
    }
    
    // Commit only this file
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(message)
        .arg("--")
        .arg(relative_path)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git commit failed",
        ));
    }
    
    Ok(())
}
