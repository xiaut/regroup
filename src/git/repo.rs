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

/// Initialize a git repository
pub fn git_init(repo_path: &Path) -> io::Result<()> {
    // Check if already a git repo
    if repo_path.join(".git").exists() {
        return Ok(());
    }

    fs::create_dir_all(repo_path)?;

    crate::ui::log("Initializing git repository...");
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path).arg("init");
    let output = quiet(cmd)?;

    if !output.status.success() {
        return Err(io::Error::other("git init failed"));
    }

    // Ensure the initial branch is "main": pull/push operations hardcode it.
    // git_init only runs on a freshly created repository (no commits yet), so
    // moving HEAD is safe even if git's default branch is "master".
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("symbolic-ref")
        .arg("--short")
        .arg("HEAD");
    let head = quiet(cmd);
    if let Ok(ref o) = head
        && o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim() != "main"
    {
        let mut cmd = Command::new("git");
        cmd.current_dir(repo_path)
            .arg("symbolic-ref")
            .arg("HEAD")
            .arg("refs/heads/main");
        let _ = quiet(cmd);
    }

    // Configure git to allow commits without user info
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg("user.email")
        .arg("regroup@local");
    let _ = quiet(cmd);

    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg("user.name")
        .arg("regroup");
    let _ = quiet(cmd);

    // Configure git to allow pushing to the currently checked-out branch
    // This is essential for decentralized sync where each device pushes to others
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg("receive.denyCurrentBranch")
        .arg("updateInstead");
    let _ = quiet(cmd);

    // Configure pull to use rebase by default for cleaner history
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg("pull.rebase")
        .arg("true");
    let _ = quiet(cmd);

    Ok(())
}
