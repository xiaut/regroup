//! Git changes detection
//!
//! This module detects unstaged changes and modified files.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Machine, DEVICES_DIR, GLOBAL_CONFIG_FILE};

/// Check if there are unstaged changes in the git repository
pub fn has_unstaged_changes(repo_path: &Path) -> bool {
    if !super::repo::is_git_repo(repo_path) {
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
            // Any non-empty porcelain line means something changed:
            // worktree modifications (" M"), staged changes ("M "), or
            // untracked files ("??"). (Ignored files never appear without
            // --ignored, so no special-casing is needed.)
            status.lines().any(|line| !line.trim().is_empty())
        }
        _ => false,
    }
}

/// Get list of modified config files (settings.json and devices/), as paths
/// relative to the repository root
pub fn get_modified_config_files(repo_path: &Path) -> Vec<PathBuf> {
    if !super::repo::is_git_repo(repo_path) {
        return Vec::new();
    }
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("status")
        .arg("--porcelain")
        .output();
    
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|line| {
                    let line = line.trim_end();
                    // Porcelain format: XY <path> ("R  old -> new" for renames)
                    if line.len() < 4 {
                        return None;
                    }
                    let xy = &line[..2];
                    if !matches!(xy, " M" | "M " | "MM" | "A " | "AM" | "D " | "R " | "C " | "??") {
                        return None;
                    }
                    let path_part = line[3..].trim().trim_matches('"');
                    if path_part.is_empty() {
                        return None;
                    }
                    // For renames keep the new path (after "-> ")
                    let path_str = path_part.rsplit(" -> ").next().unwrap_or(path_part);
                    Some(PathBuf::from(path_str))
                })
                .filter(|p| {
                    // Only track config files
                    let path_str = p.to_string_lossy();
                    path_str == GLOBAL_CONFIG_FILE
                        || path_str.starts_with(&format!("{}/", DEVICES_DIR))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Check if machines.json has been modified
pub fn is_machines_json_modified(repo_path: &Path) -> bool {
    let modified_files = get_modified_config_files(repo_path);
    modified_files.iter().any(|p| p.ends_with("machines.json"))
}

/// Get machines from machines.json if it has been modified locally
pub fn get_local_machines_changes(repo_path: &Path, machines: &[Machine]) -> Option<Vec<Machine>> {
    if !is_machines_json_modified(repo_path) {
        return None;
    }
    
    let machines_path = repo_path.join("machines.json");
    if !machines_path.exists() {
        return None;
    }
    
    match std::fs::read_to_string(&machines_path) {
        Ok(content) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<Machine>>(&content) {
                Some(parsed)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}
