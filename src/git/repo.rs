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

/// Repo-local git settings required for unattended decentralized sync.
///
/// Deliberately minimal: the `tmp/` overlay keeps unconfirmed data out of
/// the repository and `settings.json` is local-only (outside the repo), so
/// every file in `git/` has exactly one writer and no extra
/// conflict-avoidance settings are needed. What remains is only what
/// unattended, decentralized sync requires.
pub const REQUIRED_CONFIG: &[(&str, &str)] = &[
    // Commits are created automatically: never block on missing user info.
    ("user.name", "regroup"),
    ("user.email", "regroup@local"),
    // Never block on a signing prompt or an interactive editor.
    ("commit.gpgsign", "false"),
    ("core.editor", "true"),
    ("sequence.editor", "true"),
    // Peers push to this checked-out branch.
    ("receive.denyCurrentBranch", "updateInstead"),
    // The first sync between two independently bootstrapped repos has
    // unrelated histories; rebase is what makes that pull succeed and gives
    // a linear history.
    ("pull.rebase", "true"),
    // Drop remote-tracking refs of removed devices.
    ("fetch.prune", "true"),
];

/// Redundant settings written by older versions, removed on init:
/// `merge.ff` is shadowed by `pull.rebase = true`, and every push passes an
/// explicit `<remote> main` refspec, so `push.default` is never consulted.
const LEGACY_CONFIG: &[&str] = &["merge.ff", "push.default"];

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

/// Unset a repo-local git config key. Best effort.
fn unset_config(repo_path: &Path, key: &str) {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("config")
        .arg("--unset")
        .arg(key);
    let _ = quiet(cmd);
}

/// Initialize a git repository
pub fn git_init(repo_path: &Path) -> io::Result<()> {
    if !repo_path.join(".git").exists() {
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
    }

    // Apply the minimal required configuration on every run, not only on
    // first init: a repository created by an older regroup version (or
    // touched by hand) must still converge to the required set.
    for (key, value) in REQUIRED_CONFIG {
        set_config(repo_path, key, value);
    }
    for key in LEGACY_CONFIG {
        unset_config(repo_path, key);
    }

    Ok(())
}

/// Verify that the repository exists, that the required git settings are
/// configured, and that the working directory is clean.
pub fn check_repo(repo_path: &Path) -> Result<(), String> {
    if !is_git_repo(repo_path) {
        return Err(format!("{} is not a git repository", repo_path.display()));
    }

    for (key, expected) in REQUIRED_CONFIG {
        let output = Command::new("git")
            .current_dir(repo_path)
            .arg("config")
            .arg("--get")
            .arg(key)
            .output()
            .map_err(|e| format!("cannot run git config: {e}"))?;
        if !output.status.success() {
            return Err(format!("git config {key} is not set"));
        }
        let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if actual != *expected {
            return Err(format!(
                "git config {key} is {actual:?}, expected {expected:?}"
            ));
        }
    }

    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("cannot run git status: {e}"))?;
    if !output.status.success() {
        return Err("git status failed".to_string());
    }
    if !output.stdout.is_empty() {
        return Err("working directory is not clean".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("regroup-repo-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn config_get(repo: &Path, key: &str) -> Option<String> {
        let output = Command::new("git")
            .current_dir(repo)
            .arg("config")
            .arg("--get")
            .arg(key)
            .output()
            .ok()?;
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    #[test]
    fn git_init_sets_exactly_the_minimal_config() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("minimal");
        git_init(&repo).expect("git init");

        for (key, expected) in REQUIRED_CONFIG {
            assert_eq!(
                config_get(&repo, key).as_deref(),
                Some(*expected),
                "required config {key}"
            );
        }
        // Redundant settings must NOT be set.
        assert!(config_get(&repo, "merge.ff").is_none(), "merge.ff must not be set");
        assert!(
            config_get(&repo, "push.default").is_none(),
            "push.default must not be set"
        );

        // Fresh repository: clean tree, all checks pass.
        assert!(check_repo(&repo).is_ok(), "check_repo on fresh repo: {:?}", check_repo(&repo));

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn git_init_converges_an_existing_repo_and_removes_legacy_keys() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("legacy");
        git_init(&repo).expect("git init");
        // Simulate an older version's extra settings.
        set_config(&repo, "merge.ff", "only");
        set_config(&repo, "push.default", "current");
        set_config(&repo, "pull.rebase", "false");

        git_init(&repo).expect("re-init");
        assert!(config_get(&repo, "merge.ff").is_none());
        assert!(config_get(&repo, "push.default").is_none());
        assert_eq!(config_get(&repo, "pull.rebase").as_deref(), Some("true"));
        assert!(check_repo(&repo).is_ok());

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn check_repo_detects_missing_repo_dirty_tree_and_bad_settings() {
        if !git_available() {
            return;
        }
        // Not a repository at all.
        let plain = temp_repo("plain");
        assert!(check_repo(&plain).is_err());
        let _ = fs::remove_dir_all(&plain);

        let repo = temp_repo("checks");
        git_init(&repo).expect("git init");

        // Dirty working tree (untracked file).
        fs::write(repo.join("stray.txt"), "x").unwrap();
        let err = check_repo(&repo).unwrap_err();
        assert!(err.contains("not clean"), "unexpected error: {err}");
        fs::remove_file(repo.join("stray.txt")).unwrap();
        assert!(check_repo(&repo).is_ok());

        // Broken required setting.
        set_config(&repo, "pull.rebase", "false");
        let err = check_repo(&repo).unwrap_err();
        assert!(err.contains("pull.rebase"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&repo);
    }
}
