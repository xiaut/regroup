//! Git synchronization operations
//!
//! This module handles git pull, push, and sync operations.

use std::io;
use std::path::Path;

// Git commands run through crate::ui::run_logged, which puts their output
// into the one log stream: during a temporary CLI pass (direct output
// mode) the output is forwarded live to the CLI — interactive prompts
// work — and recorded for the TUI Logs pane; otherwise it is captured and
// logged line by line.

/// Pull from a specific remote. `remote_name` may also be a URL: git
/// accepts a URL wherever a remote name is expected, which is how
/// per-user sync targets (no named remote) are reached.
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

/// Push to a specific remote. `remote_name` may also be a URL (see
/// [`git_pull_remote`]).
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

/// One sync target: `target` is what git receives on the command line (a
/// remote name or a URL) and `label` is how it is shown in the logs, e.g.
/// `user@device` for a per-user sync target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncTarget {
    pub label: String,
    pub target: String,
}

/// Sync with specific targets: pull from all of them first, then push to
/// all of them. Each successful pull/push is logged through the ui
/// (activity pane in interactive mode).
///
/// Scoped syncs reach each user@device through its own URL, which needs no
/// named remote: git accepts a URL wherever a remote name is expected, so
/// every user@device can be synced — and prompt for its own password —
/// independently.
pub fn sync_with_targets(repo_path: &Path, targets: &[SyncTarget]) -> Vec<String> {
    let mut errors = Vec::new();

    if !super::repo::is_git_repo(repo_path) {
        return errors;
    }

    for t in targets {
        match git_pull_remote(repo_path, &t.target) {
            Ok(()) => {
                crate::ui::log(&format!("  - pulled from {}", t.label));
            }
            Err(e) => errors.push(format!("pull from {}: {}", t.label, e)),
        }
    }

    for t in targets {
        match git_push_remote(repo_path, &t.target) {
            Ok(()) => {
                crate::ui::log(&format!("  - pushed to {}", t.label));
            }
            Err(e) => errors.push(format!("push to {}: {}", t.label, e)),
        }
    }

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
