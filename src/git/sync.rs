//! Git synchronization operations
//!
//! This module handles git pull, push, and sync operations.

use std::io;
use std::path::Path;
use std::process::Command;

use crate::git::remote;

/// Run a git command without suppressing its output.
/// The output is shown to the user for visibility.
fn quiet(cmd: Command) -> Command {
    cmd
}

/// Pull from a specific remote
pub fn git_pull_remote(repo_path: &Path, remote_name: &str) -> io::Result<()> {
    if !super::repo::is_git_repo(repo_path) {
        return Ok(());
    }

    let status = quiet(Command::new("git"))
        .current_dir(repo_path)
        .arg("pull")
        .arg(remote_name)
        .arg("main")
        .status()?;

    if !status.success() {
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

    let status = quiet(Command::new("git"))
        .current_dir(repo_path)
        .arg("push")
        .arg(remote_name)
        .arg("main")
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "failed to push to {}",
            remote_name
        )));
    }

    Ok(())
}

/// Pull from all remotes. In verbose mode, each successful pull is logged
/// through the ui (activity pane in interactive mode).
pub fn git_pull_all(repo_path: &Path, verbose: bool) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let remotes = remote::git_list_remotes(repo_path);
    for r in &remotes {
        match git_pull_remote(repo_path, r) {
            Ok(()) => {
                if verbose {
                    crate::ui::log(&format!("  . pulled from {r}"));
                }
            }
            Err(e) => errors.push(format!("pull from {}: {}", r, e)),
        }
    }

    errors
}

/// Push to all remotes. In verbose mode, each successful push is logged
/// through the ui (activity pane in interactive mode).
pub fn git_push_all(repo_path: &Path, verbose: bool) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    let remotes = remote::git_list_remotes(repo_path);
    for r in &remotes {
        match git_push_remote(repo_path, r) {
            Ok(()) => {
                if verbose {
                    crate::ui::log(&format!("  . pushed to {r}"));
                }
            }
            Err(e) => errors.push(format!("push to {}: {}", r, e)),
        }
    }

    errors
}

/// Sync with all remotes (pull then push)
pub fn sync_with_remotes(repo_path: &Path, verbose: bool) -> Vec<String> {
    let mut errors = Vec::new();

    // Pull from all remotes first
    errors.extend(git_pull_all(repo_path, verbose));

    // Then push to all remotes
    errors.extend(git_push_all(repo_path, verbose));

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
    let add_status = quiet(Command::new("git"))
        .current_dir(repo_path)
        .arg("add")
        .arg(file_path)
        .status()?;

    if !add_status.success() {
        return Err(io::Error::other("failed to add file"));
    }

    // Commit
    let commit_status = quiet(Command::new("git"))
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(commit_message)
        .status()?;

    if !commit_status.success() {
        // If commit fails because there's nothing to commit, that's okay
        // (file might have already been committed)
        return Ok(());
    }

    Ok(())
}
