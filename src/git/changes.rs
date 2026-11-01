//! Git changes detection
//!
//! This module detects unstaged changes and modified files.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Machine;

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
            status.lines().any(|line| {
                line.starts_with("??") || 
                line.starts_with("M ") || 
                line.starts_with("A ") || 
                line.starts_with("D ") ||
                line.starts_with("R ") ||
                line.starts_with("C ") ||
                line.starts_with("!!")
            })
        }
        _ => false,
    }
}

/// Get list of modified config files (machines.json and keys/)
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
                    if line.starts_with(" M") || line.starts_with("MM") || line.starts_with("A ") || line.starts_with("D ") || line.starts_with("R ") || line.starts_with("C ") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            Some(repo_path.join(parts[1]))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .filter(|p| {
                    // Only track config files
                    let path_str = p.to_string_lossy();
                    path_str.ends_with("machines.json") || 
                    path_str.contains("/keys/")
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
