//! Git operations module for regroup
//!
//! This module handles all git-related functionality including:
//! - Repository initialization and checks
//! - Remote management
//! - Pull/push operations
//! - Change detection

pub mod remote;
pub mod repo;
pub mod sync;

// Re-export commonly used functions for backward compatibility
pub use remote::sync_git_remotes;
pub use repo::{git_init, is_git_repo};
pub use sync::sync_with_remotes;
