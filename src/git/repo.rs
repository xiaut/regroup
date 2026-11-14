//! Git repository operations
//!
//! This module handles basic git repository operations.

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

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

/// Check if a directory is a git repository
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Set a repo-local git config key. Best effort: a key that cannot be set
/// never prevents the repository from being usable.
fn set_config(repo_path: &Path, key: &str, value: &str) {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg(key)
        .arg(value);
    let _ = quiet(cmd);
}

/// Initialize a git repository
pub fn git_init(repo_path: &Path) -> io::Result<()> {
    // Check if already a git repo
    if repo_path.join(".git").exists() {
        return Ok(());
    }

    fs::create_dir_all(repo_path)?;

    crate::ui::log("Initializing git repository...");
    // Prefer `init -b main` (git >= 2.28): the initial branch is set
    // atomically and pull/push operations hardcode "main".
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path).arg("init").arg("-b").arg("main");
    let output = quiet(cmd);
    if !output.is_ok_and(|o| o.status.success()) {
        // Fallback for git < 2.28: init with the default branch, then move
        // HEAD to main. Safe here: the repository has no commits yet.
        let mut cmd = Command::new("git");
        cmd.current_dir(repo_path).arg("init");
        let output = quiet(cmd)?;
        if !output.status.success() {
            return Err(io::Error::other("git init failed"));
        }
        let mut cmd = Command::new("git");
        cmd.current_dir(repo_path)
            .arg("symbolic-ref")
            .arg("HEAD")
            .arg("refs/heads/main");
        let _ = quiet(cmd);
    }

    // Commits are created automatically by regroup, so they must never block
    // on missing user info, GPG signing, or an interactive editor.
    set_config(repo_path, "user.email", "regroup@local");
    set_config(repo_path, "user.name", "regroup");
    set_config(repo_path, "commit.gpgsign", "false");
    set_config(repo_path, "core.editor", "true");
    set_config(repo_path, "sequence.editor", "true");

    // Decentralized sync: every device pushes to every other device's
    // currently checked-out branch, so allow that on the receiving side.
    set_config(repo_path, "receive.denyCurrentBranch", "updateInstead");

    // Sync shape: rebase pulls for a linear history (local commits are
    // replayed on top of the remote, i.e. last-writer-wins ordering),
    // fast-forward-only merges, and prune remote-tracking refs of removed
    // devices on fetch.
    set_config(repo_path, "pull.rebase", "true");
    set_config(repo_path, "merge.ff", "only");
    set_config(repo_path, "push.default", "current");
    set_config(repo_path, "fetch.prune", "true");

    Ok(())
}
