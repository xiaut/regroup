//! Git synchronization operations
//!
//! This module handles git pull, push, and sync operations.

use std::io;
use std::path::Path;

use crate::git::remote;

// Git commands run through crate::ui::run_logged, which puts their output
// into the one log stream: during a temporary CLI pass (direct output
// mode) the output is forwarded live to the CLI — interactive prompts
// work — and recorded for the TUI Logs pane; otherwise it is captured and
// logged line by line.

/// Pull from a specific remote
pub fn git_pull_remote(repo_path: &Path, remote_name: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    crate::ui::log(&format!(
        "Pulling from git remote '{}' (may prompt for password)...",
        remote_name
    ));
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(repo_path)
        .arg("pull")
        .arg(remote_name)
        .arg("main");
    let output = crate::ui::run_logged(cmd)?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "failed to pull from {}",
            remote_name
        )));
    }

    Ok(())
}

/// Push to a specific remote
pub fn git_push_remote(repo_path: &Path, remote_name: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    crate::ui::log(&format!(
        "Pushing to git remote '{}' (may prompt for password)...",
        remote_name
    ));
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(repo_path)
        .arg("push")
        .arg(remote_name)
        .arg("main");
    let output = crate::ui::run_logged(cmd)?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "failed to push to {}",
            remote_name
        )));
    }

    Ok(())
}

/// Pull from all remotes. Each successful pull is logged through the ui
/// (activity pane in interactive mode).
pub fn git_pull_all(repo_path: &Path) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let remotes = remote::git_list_remotes(repo_path);
    for r in &remotes {
        match git_pull_remote(repo_path, r) {
            Ok(()) => {
                crate::ui::log(&format!("  - pulled from {r}"));
            }
            Err(e) => errors.push(format!("pull from {}: {}", r, e)),
        }
    }

    errors
}

/// Push to all remotes. Each successful push is logged through the ui
/// (activity pane in interactive mode).
pub fn git_push_all(repo_path: &Path) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let remotes = remote::git_list_remotes(repo_path);
    for r in &remotes {
        match git_push_remote(repo_path, r) {
            Ok(()) => {
                crate::ui::log(&format!("  - pushed to {r}"));
            }
            Err(e) => errors.push(format!("push to {}: {}", r, e)),
        }
    }

    errors
}

/// Sync with all remotes (pull then push)
pub fn sync_with_remotes(repo_path: &Path) -> Vec<String> {
    let mut errors = Vec::new();

    // Pull from all remotes first
    errors.extend(git_pull_all(repo_path));

    // Then push to all remotes
    errors.extend(git_push_all(repo_path));

    errors
}

/// Add and commit a file
pub fn git_add_and_commit_file(
    repo_path: &Path,
    file_path: &Path,
    commit_message: &str,
) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    // Add the file
    let mut add_cmd = std::process::Command::new("git");
    add_cmd.current_dir(repo_path)
        .arg("add")
        .arg(file_path);
    let add_output = crate::ui::run_logged(add_cmd)?;

    if !add_output.status.success() {
        return Err(io::Error::other("failed to add file"));
    }

    // Commit
    let mut commit_cmd = std::process::Command::new("git");
    commit_cmd.current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(commit_message);
    let commit_output = crate::ui::run_logged(commit_cmd)?;

    if !commit_output.status.success() {
        // If commit fails because there's nothing to commit, that's okay
        // (file might have already been committed)
        return Ok(());
    }

    Ok(())
}
