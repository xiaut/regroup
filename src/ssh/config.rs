//! SSH configuration module for regroup
//!
//! This module handles the generation and updating of SSH configuration files.

use std::fs;
use std::io;
use std::path::Path;

// ============================================================
// ~/.ssh/config
// ============================================================

/// Replaces ONLY the "Host <name>" block; all other blocks are preserved
/// (e.g. github.com). Idempotent: running it twice yields the same file.
///
/// The block carries `HostName` (and `Port` when `port_explicit`), plus the
/// credentials of the local user's outgoing connections to this device when
/// known: one `User` line (ssh uses the first `User` value it finds) and one
/// `IdentityFile` line per connection (ssh offers ALL IdentityFile keys, so
/// every user@device connection gets its own key tried — this is what keeps
/// git sync over `user@device` from prompting for a password once a key was
/// installed).
pub fn update_block(
    path: &Path,
    name: &str,
    ip: &str,
    port: u16,
    port_explicit: bool,
    user: Option<&str>,
    identity_files: &[String],
) -> io::Result<()> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut in_block = false;
    for line in old.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() && words[0] == "Host" {
            in_block = false;
            if words[1..].contains(&name) {
                in_block = true;
                continue;
            }
        }
        if in_block {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let body = kept.trim_end();
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
    }
    out.push_str(&format!("Host {}\n    HostName {}\n", name, ip));
    if port_explicit {
        out.push_str(&format!("    Port {}\n", port));
    }
    if let Some(user) = user {
        out.push_str(&format!("    User {}\n", user));
    }
    for identity_file in identity_files {
        out.push_str(&format!("    IdentityFile {}\n", identity_file));
    }
    fs::write(path, out)
}

/// Remove the "Host <name>" block from ~/.ssh/config. All other blocks are
/// preserved. Returns true when a block was found and removed.
///
/// Safe: a device can only be accepted under a name that was not already
/// used by an existing block (see device_name_taken), so a block bearing a
/// known device's name was necessarily written by regroup.
pub fn remove_block(path: &Path, name: &str) -> io::Result<bool> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut in_block = false;
    let mut found = false;
    for line in old.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() && words[0] == "Host" {
            in_block = words[1..].contains(&name);
            if in_block {
                found = true;
                continue;
            }
        }
        if in_block {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    if found {
        let out = format!("{}\n", kept.trim_end());
        fs::write(path, out)?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("regroup-ssh-cfg-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        (dir, path)
    }

    #[test]
    fn update_block_writes_user_and_identity_files() {
        let (dir, path) = test_config("update-creds");
        fs::write(&path, "Host keep\n    HostName 1.1.1.1\n").unwrap();

        let keys = vec![
            "/home/alice/.ssh/id_regroup_alice@laptop".to_string(),
            "/home/alice/.ssh/id_regroup_alice@laptop".to_string(), // duplicate
        ];
        let keys: Vec<String> = {
            let mut k = keys;
            k.sort();
            k.dedup();
            k
        };
        update_block(&path, "laptop", "2.2.2.2", 2222, true, Some("bob"), &keys).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Host keep"), "unrelated block kept");
        assert_eq!(
            content,
            "Host keep\n    HostName 1.1.1.1\n\nHost laptop\n    HostName 2.2.2.2\n    Port 2222\n    User bob\n    IdentityFile /home/alice/.ssh/id_regroup_alice@laptop\n"
        );

        // Idempotent: running it again yields the same file.
        update_block(&path, "laptop", "2.2.2.2", 2222, true, Some("bob"), &keys).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), content);

        // A second update without credentials drops the credential lines
        // (e.g. after the last connection to the device was removed).
        update_block(&path, "laptop", "2.2.2.2", 2222, true, None, &[]).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("User bob"));
        assert!(!content.contains("IdentityFile"));
        assert!(content.contains("Host laptop\n    HostName 2.2.2.2\n    Port 2222\n"));

        // Port line omitted when not explicit.
        update_block(&path, "laptop", "2.2.2.2", 22, false, None, &[]).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("Port"));
        assert!(content.contains("HostName 2.2.2.2"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_block_replaces_an_existing_block_with_credentials() {
        let (dir, path) = test_config("update-replace");
        fs::write(
            &path,
            "Host laptop\n    HostName 9.9.9.9\n    User old\n\nHost other\n    HostName 3.3.3.3\n",
        )
        .unwrap();
        let keys = vec!["/home/alice/.ssh/id_regroup_alice@laptop".to_string()];
        update_block(&path, "laptop", "2.2.2.2", 22, false, Some("bob"), &keys).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("9.9.9.9"), "old block replaced");
        assert!(!content.contains("User old"), "old user replaced");
        assert!(content.contains("Host other\n    HostName 3.3.3.3\n"), "other block kept");
        assert!(content.contains("Host laptop\n    HostName 2.2.2.2\n    User bob\n    IdentityFile /home/alice/.ssh/id_regroup_alice@laptop\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_block_removes_only_the_named_block() {
        let dir = std::env::temp_dir().join(format!("regroup-ssh-cfg-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        fs::write(
            &path,
            "Host keep\n    HostName 1.1.1.1\n\nHost gone\n    HostName 2.2.2.2\n    User bob\n\nHost multi other\n    HostName 3.3.3.3\n",
        )
        .unwrap();

        assert!(remove_block(&path, "gone").unwrap());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Host keep"), "unrelated block kept");
        assert!(!content.contains("2.2.2.2"), "named block removed");

        // Idempotent: a second call finds nothing.
        assert!(!remove_block(&path, "gone").unwrap());

        // A multi-pattern Host line matches any of its names, and the
        // whole block is removed.
        assert!(remove_block(&path, "other").unwrap());
        assert!(!fs::read_to_string(&path).unwrap().contains("3.3.3.3"));

        let _ = fs::remove_dir_all(&dir);
    }
}
