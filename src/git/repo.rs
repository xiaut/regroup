//! Git repository operations
//!
//! This module handles basic git repository operations.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

/// Check if a directory is a git repository
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Initialize a git repository
pub fn git_init(repo_path: &Path) -> io::Result<()> {
    // Check if already a git repo
    if repo_path.join(".git").exists() {
        return Ok(());
    }

    fs::create_dir_all(repo_path)?;

    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("init")
        .status()?;

    if !status.success() {
        return Err(io::Error::other("git init failed"));
    }

    // Ensure the initial branch is "main": pull/push operations hardcode it.
    // git_init only runs on a freshly created repository (no commits yet), so
    // moving HEAD is safe even if git's default branch is "master".
    let head = Command::new("git")
        .current_dir(repo_path)
        .arg("symbolic-ref")
        .arg("--short")
        .arg("HEAD")
        .output();
    if let Ok(o) = head
        && o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim() != "main"
    {
        let _ = Command::new("git")
            .current_dir(repo_path)
            .arg("symbolic-ref")
            .arg("HEAD")
            .arg("refs/heads/main")
            .status();
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
