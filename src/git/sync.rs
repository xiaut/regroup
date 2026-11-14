//! Git synchronization operations
//!
//! This module handles git pull, push, and sync operations.

use std::io;
use std::path::Path;
use std::process::Stdio;

use crate::git::remote;

/// Run a git command and log its output.
/// When in direct output mode (temporary CLI mode), stderr is passed through
/// directly to allow interactive prompts (e.g., password) to work.
fn quiet(mut cmd: std::process::Command) -> io::Result<std::process::Output> {
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
    let output = quiet(cmd)?;

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
    let output = quiet(cmd)?;

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
    let add_output = quiet(add_cmd)?;

    if !add_output.status.success() {
        return Err(io::Error::other("failed to add file"));
    }

    // Commit
    let mut commit_cmd = std::process::Command::new("git");
    commit_cmd.current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(commit_message);
    let commit_output = quiet(commit_cmd)?;

    if !commit_output.status.success() {
        // If commit fails because there's nothing to commit, that's okay
        // (file might have already been committed)
        return Ok(());
    }

    Ok(())
}
