//! Git operations module for regroup
//!
//! This module handles all git-related functionality including:
//! - Repository initialization and checks
//! - Remote management
//! - Pull/push operations
//! - Change detection

pub mod repo;
pub mod remote;
pub mod sync;
pub mod changes;

// Re-export commonly used functions for backward compatibility
pub use repo::{is_git_repo, git_init};
pub use remote::refresh_git_remotes;
pub use sync::{sync_with_remotes, git_add_and_commit_file};
pub use changes::has_unstaged_changes;
